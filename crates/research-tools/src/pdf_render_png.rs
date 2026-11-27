//! Pure-Rust bounded PNG codec, scanline predictor reversal, pixel converters, and cropper
//! for PDF page rendering (package F14b).

use crate::pdf_render::{CropRect, RenderLimits};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};

pub(crate) const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Supported 8-bit-per-channel PNG color models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ColorType {
    Gray8 = 0,
    Rgb8 = 2,
    Rgba8 = 6,
}

impl ColorType {
    pub(crate) fn channels(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

/// Uncompressed 8-bit raster pixel buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawImage {
    pub width: u32,
    pub height: u32,
    pub color_type: ColorType,
    pub pixels: Vec<u8>,
}

/// Standard IEEE 802.3 CRC-32 over one or more byte slices.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &byte in *part {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }
    !crc
}

fn read_u32_be(slice: &[u8]) -> Result<u32, String> {
    let arr: [u8; 4] = slice.try_into().map_err(|_| "truncated PNG u32".to_string())?;
    Ok(u32::from_be_bytes(arr))
}

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) -> Result<(), String> {
    let len = u32::try_from(data.len()).map_err(|_| "PNG chunk exceeds u32 length".to_string())?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[chunk_type, data]).to_be_bytes());
    Ok(())
}

/// Encode a [`RawImage`] into a valid PNG byte buffer, enforcing `max_png_bytes`.
pub(crate) fn encode_png(img: &RawImage, max_png_bytes: usize) -> Result<Vec<u8>, String> {
    if img.width == 0 || img.height == 0 {
        return Err("PNG dimensions must be non-zero".into());
    }
    let w = usize::try_from(img.width).map_err(|_| "PNG width overflow".to_string())?;
    let h = usize::try_from(img.height).map_err(|_| "PNG height overflow".to_string())?;
    let row_bytes = w.checked_mul(img.color_type.channels()).ok_or("PNG row size overflow")?;
    let expected = row_bytes.checked_mul(h).ok_or("PNG pixel buffer overflow")?;
    if img.pixels.len() != expected {
        return Err(format!("PNG pixel length mismatch: got {}, expected {expected}", img.pixels.len()));
    }

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&img.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&img.height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = img.color_type as u8;

    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    for row in 0..h {
        let start = row * row_bytes;
        enc.write_all(&[0u8])
            .and_then(|()| enc.write_all(&img.pixels[start..start + row_bytes]))
            .map_err(|e| format!("PNG zlib compression failed: {e}"))?;
    }
    let idat = enc.finish().map_err(|e| format!("PNG zlib finish failed: {e}"))?;
    let total = 8 + 25 + 12 + idat.len() + 12;
    if total > max_png_bytes {
        return Err(format!("encoded PNG ({total} bytes) exceeds limit ({max_png_bytes} bytes)"));
    }

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(PNG_SIGNATURE);
    write_chunk(&mut out, b"IHDR", &ihdr)?;
    write_chunk(&mut out, b"IDAT", &idat)?;
    write_chunk(&mut out, b"IEND", &[])?;
    if out.len() > max_png_bytes {
        return Err(format!("encoded PNG ({} bytes) exceeds limit ({max_png_bytes} bytes)", out.len()));
    }
    Ok(out)
}

/// Fast PNG signature and IHDR dimension reader without decompressing pixel data.
pub(crate) fn inspect_png_header(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 33 || &bytes[..8] != PNG_SIGNATURE {
        return Err("invalid or truncated PNG signature".into());
    }
    if read_u32_be(&bytes[8..12])? != 13 || &bytes[12..16] != b"IHDR" {
        return Err("first PNG chunk must be a 13-byte IHDR".into());
    }
    let ihdr = &bytes[16..29];
    if crc32(&[b"IHDR", ihdr]) != read_u32_be(&bytes[29..33])? {
        return Err("PNG IHDR CRC mismatch".into());
    }
    let width = read_u32_be(&ihdr[0..4])?;
    let height = read_u32_be(&ihdr[4..8])?;
    if width == 0 || height == 0 {
        return Err("PNG header declares zero width or height".into());
    }
    Ok((width, height))
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let (ai, bi, ci) = (i32::from(a), i32::from(b), i32::from(c));
    let p = ai + bi - ci;
    let (pa, pb, pc) = ((p - ai).abs(), (p - bi).abs(), (p - ci).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Reverse PNG scanline filters (0=None, 1=Sub, 2=Up, 3=Average, 4=Paeth).
///
/// Each of `height` rows in `raw` starts with 1 filter byte followed by `width * bpp` bytes.
/// Also used for PDF `/FlateDecode` streams with `/DecodeParms << /Predictor 10..=15 >>`.
pub(crate) fn reverse_png_predictor(
    raw: &[u8],
    width: usize,
    height: usize,
    bpp: usize,
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || bpp == 0 {
        return Err("PNG predictor parameters must be non-zero".into());
    }
    let row_bytes = width.checked_mul(bpp).ok_or("PNG predictor row overflow")?;
    let stride = row_bytes.checked_add(1).ok_or("PNG predictor stride overflow")?;
    let expected = stride.checked_mul(height).ok_or("PNG predictor input overflow")?;
    if raw.len() != expected {
        return Err(format!("PNG predictor length mismatch: got {}, expected {expected}", raw.len()));
    }
    let out_len = row_bytes.checked_mul(height).ok_or("PNG predictor output overflow")?;
    let mut out = vec![0u8; out_len];

    for y in 0..height {
        let in_off = y * stride;
        let filter = raw[in_off];
        let src = &raw[in_off + 1..in_off + stride];
        let out_off = y * row_bytes;
        let prev_off = y.saturating_sub(1) * row_bytes;
        for x in 0..row_bytes {
            let a = if x >= bpp { out[out_off + x - bpp] } else { 0 };
            let b = if y > 0 { out[prev_off + x] } else { 0 };
            let c = if y > 0 && x >= bpp { out[prev_off + x - bpp] } else { 0 };
            out[out_off + x] = match filter {
                0 => src[x],
                1 => src[x].wrapping_add(a),
                2 => src[x].wrapping_add(b),
                3 => src[x].wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => src[x].wrapping_add(paeth_predictor(a, b, c)),
                other => return Err(format!("unsupported PNG scanline filter {other}")),
            };
        }
    }
    Ok(out)
}

/// Unpack MSB-first 1-bit packed monochrome scanlines into 8-bit grayscale pixels.
pub(crate) fn unpack_1bit_gray(
    packed: &[u8],
    width: usize,
    height: usize,
    row_bytes: usize,
    invert: bool,
) -> Result<Vec<u8>, String> {
    let cap = width.checked_mul(height).ok_or("1-bit gray output overflow")?;
    let mut out = Vec::with_capacity(cap);
    for y in 0..height {
        let row = &packed[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..width {
            let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
            let lit = if invert { bit == 0 } else { bit != 0 };
            out.push(if lit { 255 } else { 0 });
        }
    }
    Ok(out)
}

/// Convert 8-bit CMYK pixel buffer to 8-bit RGB, handling optional inverted `/Decode`.
pub(crate) fn cmyk_to_rgb(samples: &[u8], pixel_count: usize, invert: bool) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixel_count.saturating_mul(3));
    for px in samples.as_chunks::<4>().0 {
        let (c, m, y, k) = if invert {
            (255 - px[0], 255 - px[1], 255 - px[2], 255 - px[3])
        } else {
            (px[0], px[1], px[2], px[3])
        };
        let inv_k = u32::from(255 - k);
        rgb.push(((u32::from(255 - c) * inv_k) / 255) as u8);
        rgb.push(((u32::from(255 - m) * inv_k) / 255) as u8);
        rgb.push(((u32::from(255 - y) * inv_k) / 255) as u8);
    }
    rgb
}

/// Expand 1/4/8-bit `/Indexed /DeviceRGB` samples into 8-bit RGB pixels.
pub(crate) fn indexed_rgb_to_rgb(
    samples: &[u8],
    width: usize,
    height: usize,
    row_bytes: usize,
    bpc: u8,
    hival: u8,
    palette: &[u8],
) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(width.saturating_mul(height).saturating_mul(3));
    for y in 0..height {
        let row = &samples[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..width {
            let idx = match bpc {
                8 => row[x],
                4 => if x % 2 == 0 { row[x / 2] >> 4 } else { row[x / 2] & 0x0F },
                _ => (row[x / 8] >> (7 - (x % 8))) & 1,
            }
            .min(hival);
            let off = usize::from(idx) * 3;
            rgb.extend_from_slice(&palette[off..off + 3]);
        }
    }
    rgb
}

/// Decode a bounded PNG image (8-bit Gray/RGB/RGBA and 1-bit Gray), enforcing dimension
/// and pixel caps before decompression to defeat zip bombs.
pub(crate) fn decode_png(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
    max_pixels: u64,
) -> Result<RawImage, String> {
    if bytes.len() < 33 || &bytes[..8] != PNG_SIGNATURE {
        return Err("invalid or truncated PNG input".into());
    }
    let (mut pos, mut ihdr_info, mut idat, mut saw_iend) = (8usize, None, Vec::new(), false);

    while pos < bytes.len() {
        let header_end = pos.checked_add(8).ok_or("PNG offset overflow")?;
        if header_end > bytes.len() {
            return Err("truncated PNG chunk header".into());
        }
        let chunk_len = read_u32_be(&bytes[pos..pos + 4])? as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        let data_end = header_end.checked_add(chunk_len).ok_or("PNG chunk length overflow")?;
        let crc_end = data_end.checked_add(4).ok_or("PNG chunk CRC overflow")?;
        if crc_end > bytes.len() {
            return Err("truncated PNG chunk body".into());
        }
        let data = &bytes[header_end..data_end];
        if crc32(&[ctype, data]) != read_u32_be(&bytes[data_end..crc_end])? {
            return Err(format!("PNG chunk `{}` CRC mismatch", String::from_utf8_lossy(ctype)));
        }
        if ctype == b"IHDR" {
            if ihdr_info.is_some() || pos != 8 || chunk_len != 13 {
                return Err("malformed or duplicate PNG IHDR chunk".into());
            }
            let (width, height) = (read_u32_be(&data[0..4])?, read_u32_be(&data[4..8])?);
            if width == 0 || height == 0 {
                return Err("PNG dimensions must be non-zero".into());
            }
            if width > max_width || height > max_height {
                return Err(format!("PNG {width}x{height} exceeds limit {max_width}x{max_height}"));
            }
            let total_px = u64::from(width) * u64::from(height);
            if total_px > max_pixels {
                return Err(format!("PNG pixel count ({total_px}) exceeds limit ({max_pixels})"));
            }
            if data[10] != 0 || data[11] != 0 || data[12] != 0 {
                return Err("unsupported PNG compression, filter method, or interlacing".into());
            }
            ihdr_info = Some((width, height, data[8], data[9]));
        } else if ctype == b"IDAT" {
            if ihdr_info.is_none() {
                return Err("PNG IDAT encountered before IHDR".into());
            }
            idat.extend_from_slice(data);
        } else if ctype == b"IEND" {
            saw_iend = true;
            break;
        }
        pos = crc_end;
    }

    let (width, height, bit_depth, color_code) = ihdr_info.ok_or("PNG missing IHDR chunk")?;
    if !saw_iend || idat.is_empty() {
        return Err("PNG missing IDAT or IEND chunk".into());
    }
    let (color_type, is_1bit) = match (bit_depth, color_code) {
        (8, 0) => (ColorType::Gray8, false),
        (8, 2) => (ColorType::Rgb8, false),
        (8, 6) => (ColorType::Rgba8, false),
        (1, 0) => (ColorType::Gray8, true),
        _ => return Err(format!("unsupported PNG bit_depth={bit_depth} color_type={color_code}")),
    };

    let w = usize::try_from(width).map_err(|_| "PNG width overflow".to_string())?;
    let h = usize::try_from(height).map_err(|_| "PNG height overflow".to_string())?;
    let (pred_w, bpp) = if is_1bit { (w.div_ceil(8), 1) } else { (w, color_type.channels()) };
    let row_bytes = pred_w.checked_mul(bpp).ok_or("PNG row size overflow")?;
    let expected = row_bytes.checked_add(1).and_then(|s| s.checked_mul(h)).ok_or("PNG size overflow")?;
    let raw_filtered = inflate_zlib_bounded(&idat, expected)?;
    if raw_filtered.len() < expected {
        return Err(format!("PNG IDAT truncated: got {}, expected {expected}", raw_filtered.len()));
    }

    let unfiltered = reverse_png_predictor(&raw_filtered, pred_w, h, bpp)?;
    let pixels = if is_1bit { unpack_1bit_gray(&unfiltered, w, h, row_bytes, false)? } else { unfiltered };
    Ok(RawImage { width, height, color_type, pixels })
}

/// Decompress a zlib stream with a strict upper bound to defeat zip bombs.
pub(crate) fn inflate_zlib_bounded(data: &[u8], max_bytes: usize) -> Result<Vec<u8>, String> {
    let cap = u64::try_from(max_bytes).ok().and_then(|n| n.checked_add(1)).ok_or("zlib bound overflow")?;
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(cap)
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib decompression failed: {e}"))?;
    if out.len() > max_bytes {
        return Err(format!("decompressed payload exceeds bounded limit ({max_bytes} bytes)"));
    }
    Ok(out)
}

/// Crop an encoded PNG to normalized `crop` coordinates and re-encode under `limits`.
pub(crate) fn crop_png(
    png_bytes: &[u8],
    crop: &CropRect,
    limits: &RenderLimits,
) -> Result<(Vec<u8>, u32, u32), String> {
    let crop = crop.validate()?;
    let img = decode_png(png_bytes, limits.max_width, limits.max_height, limits.max_pixels)?;
    let (wf, hf) = (f64::from(img.width), f64::from(img.height));
    let px = ((crop.x * wf).floor().max(0.0) as u32).min(img.width.saturating_sub(1));
    let py = ((crop.y * hf).floor().max(0.0) as u32).min(img.height.saturating_sub(1));
    let pw = ((crop.w * wf).round().max(1.0) as u32).min(img.width - px).max(1);
    let ph = ((crop.h * hf).round().max(1.0) as u32).min(img.height - py).max(1);

    let bpp = img.color_type.channels();
    let (src_stride, dst_stride) = ((img.width as usize) * bpp, (pw as usize) * bpp);
    let mut cropped = Vec::with_capacity(dst_stride * (ph as usize));
    for row in 0..(ph as usize) {
        let start = ((py as usize) + row) * src_stride + (px as usize) * bpp;
        cropped.extend_from_slice(&img.pixels[start..start + dst_stride]);
    }
    let out = encode_png(
        &RawImage { width: pw, height: ph, color_type: img.color_type, pixels: cropped },
        limits.max_png_bytes,
    )?;
    Ok((out, pw, ph))
}
