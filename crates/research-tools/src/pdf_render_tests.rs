//! Unit tests for `pdf_render.rs` and `pdf_render_png.rs` (package F14b).

use super::*;
use crate::pdf_render_png::{
    crop_png, decode_png, encode_png, reverse_png_predictor, ColorType, RawImage, PNG_SIGNATURE,
};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn build_pdf(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj", idx + 1).unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1).unwrap();
    for off in offsets {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF",
        objects.len() + 1
    )
    .unwrap();
    out
}

fn stream_obj(dict_body: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict_body} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn build_image_pdf(
    w: u32,
    h: u32,
    color_space: &str,
    bpc: u8,
    filter: Option<&str>,
    extra_dict: &str,
    data: &[u8],
) -> Vec<u8> {
    let filt = filter.map(|f| format!("/Filter {f}")).unwrap_or_default();
    let img_dict = format!(
        "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {color_space} /BitsPerComponent {bpc} {filt} {extra_dict}"
    );
    let content = b"q 612 0 0 792 0 0 cm /Im0 Do Q";
    let objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        stream_obj(&img_dict, data),
        stream_obj("", content),
    ];
    build_pdf(&objs, "")
}

fn build_multipage_gray_pdf(pages: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let mut objs = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new()];
    let mut kids = Vec::new();
    for &(w, h, pixels) in pages {
        let page_num = objs.len() + 1;
        let img_num = page_num + 1;
        let cnt_num = page_num + 2;
        kids.push(format!("{page_num} 0 R"));
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 {img_num} 0 R >> >> /Contents {cnt_num} 0 R >>"
            )
            .into_bytes(),
        );
        let compressed = zlib_compress(pixels);
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode"
        );
        objs.push(stream_obj(&dict, &compressed));
        objs.push(stream_obj("", b"q 612 0 0 792 0 0 cm /Im0 Do Q"));
    }
    objs[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes();
    build_pdf(&objs, "")
}

fn build_vector_text_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [5 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream_obj("", stream.as_bytes()),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".to_vec(),
    ];
    build_pdf(&objs, "")
}

fn build_oversized_dim_pdf(w: u32, h: u32) -> Vec<u8> {
    build_image_pdf(w, h, "/DeviceGray", 8, Some("/FlateDecode"), "", &zlib_compress(&[0u8; 4]))
}

fn build_encrypted_pdf() -> Vec<u8> {
    let stream = b"BT /F1 12 Tf 72 720 Td (secret) Tj ET";
    let objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [5 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream_obj("", stream),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".to_vec(),
        b"<< /Filter /Standard /V 2 /R 3 /Length 128 /P -4 /O <000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f> /U <1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100> >>".to_vec(),
    ];
    build_pdf(&objs, "/Encrypt 6 0 R /ID [<00112233445566778899aabbccddeeff> <00112233445566778899aabbccddeeff>]")
}

fn png_crc32(chunk_type: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn make_filtered_png(w: u32, h: u32, color_code: u8, filtered_rows: &[u8]) -> Vec<u8> {
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = color_code;
    let idat = zlib_compress(filtered_rows);
    let mut out = PNG_SIGNATURE.to_vec();
    for (ct, data) in [(b"IHDR", ihdr.as_slice()), (b"IDAT", idat.as_slice()), (b"IEND", b"".as_slice())] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(ct);
        out.extend_from_slice(data);
        out.extend_from_slice(&png_crc32(ct, data).to_be_bytes());
    }
    out
}

#[test]
fn png_roundtrip_gray_rgb_rgba_and_bounds() {
    let cases = [
        RawImage { width: 3, height: 2, color_type: ColorType::Gray8, pixels: vec![0, 64, 128, 192, 220, 255] },
        RawImage { width: 2, height: 2, color_type: ColorType::Rgb8, pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 12, 34, 56] },
        RawImage { width: 2, height: 1, color_type: ColorType::Rgba8, pixels: vec![10, 20, 30, 255, 40, 50, 60, 128] },
    ];
    for img in &cases {
        let png = encode_png(img, MAX_RENDER_PNG_BYTES).unwrap();
        assert_eq!(&png[..8], PNG_SIGNATURE);
        assert_eq!(inspect_png_dimensions(&png).unwrap(), (img.width, img.height));
        let decoded = decode_png(&png, 100, 100, 10_000).unwrap();
        assert_eq!(&decoded, img);
    }
    // Enforce max_png_bytes on encode and dimension bounds on decode before allocation.
    assert!(encode_png(&cases[1], 10).is_err());
    let big_hdr = make_filtered_png(9000, 9000, 0, &[0u8; 4]);
    assert!(decode_png(&big_hdr, 8192, 8192, 1_000_000).unwrap_err().contains("exceeds limit"));
}

#[test]
fn png_all_five_scanline_filters_reconstruct_exact_pixels() {
    // Hand-computed 2x2 Gray8 vectors for Sub(1)+Average(3) and Sub(1)+Paeth(4).
    assert_eq!(reverse_png_predictor(&[1, 10, 20, 3, 15, 25], 2, 2, 1).unwrap(), vec![10, 30, 20, 50]);
    assert_eq!(reverse_png_predictor(&[1, 10, 20, 4, 10, 20], 2, 2, 1).unwrap(), vec![10, 30, 20, 50]);

    // 3x5 RGB image exercising all 5 filters (0=None, 1=Sub, 2=Up, 3=Average, 4=Paeth) via decode_png.
    let (w, h, bpp) = (3usize, 5usize, 3usize);
    let mut pixels = Vec::with_capacity(w * h * bpp);
    for y in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(&[((y * 41 + x * 17) % 256) as u8, ((y * 23 + x * 61) % 256) as u8, ((y * 97 + x * 13) % 256) as u8]);
        }
    }
    let row_bytes = w * bpp;
    let mut filtered = Vec::with_capacity(h * (row_bytes + 1));
    for y in 0..h {
        let f = y as u8;
        filtered.push(f);
        for x in 0..row_bytes {
            let cur = pixels[y * row_bytes + x];
            let a = if x >= bpp { pixels[y * row_bytes + x - bpp] } else { 0 };
            let b = if y > 0 { pixels[(y - 1) * row_bytes + x] } else { 0 };
            let c = if y > 0 && x >= bpp { pixels[(y - 1) * row_bytes + x - bpp] } else { 0 };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                _ => {
                    let (ai, bi, ci) = (i32::from(a), i32::from(b), i32::from(c));
                    let p = ai + bi - ci;
                    let (pa, pb, pc) = ((p - ai).abs(), (p - bi).abs(), (p - ci).abs());
                    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                }
            };
            filtered.push(cur.wrapping_sub(pred));
        }
    }
    let png = make_filtered_png(w as u32, h as u32, 2, &filtered);
    let decoded = decode_png(&png, 64, 64, 4096).unwrap();
    assert_eq!(decoded.pixels, pixels);
}

#[test]
fn crop_rect_validation_clamping_and_png_slicing() {
    let parsed = CropRect::parse("0.25, 0.5, 0.5, 0.5").unwrap();
    assert_eq!(parsed, CropRect { x: 0.25, y: 0.5, w: 0.5, h: 0.5 });
    let clamped = CropRect::new(0.5, 0.5, 0.5005, 0.5005).unwrap();
    assert!((clamped.w - 0.5).abs() < 1e-12 && (clamped.h - 0.5).abs() < 1e-12);

    for bad in ["0,0,0,1", "-0.1,0,0.5,0.5", "1.0,0,0.1,0.1", "0.5,0.5,0.6,0.2", "NaN,0,0.5,0.5", "inf,0,0.5,0.5", "0.1,0.2,0.3"] {
        assert!(CropRect::parse(bad).is_err(), "expected invalid crop `{bad}` to fail");
    }

    // 4x4 Gray8 image with pixel value = y*10 + x.
    let mut px = Vec::with_capacity(16);
    for y in 0..4u8 {
        for x in 0..4u8 {
            px.push(y * 10 + x);
        }
    }
    let full = encode_grayscale_png(4, 4, &px).unwrap();
    let (cropped_png, cw, ch) = crop_png(&full, &CropRect::new(0.25, 0.5, 0.5, 0.5).unwrap(), &RenderLimits::default()).unwrap();
    assert_eq!((cw, ch), (2, 2));
    assert_eq!(decode_png(&cropped_png, 16, 16, 256).unwrap().pixels, vec![21, 22, 31, 32]);

    // Tiny corner crop clamps safely to at least 1x1 within bounds.
    let (_, tw, th) = crop_png(&full, &CropRect::new(0.99, 0.99, 0.001, 0.001).unwrap(), &RenderLimits::default()).unwrap();
    assert_eq!((tw, th), (1, 1));
}

#[test]
fn raster_xobject_extracts_1bit_gray_rgb_cmyk_indexed_and_predicted() {
    let renderer = RasterXObjectRenderer::default();

    // 1) 1-bit monochrome FlateDecode (default [0 1] and inverted /Decode [1 0]).
    let mono_packed = zlib_compress(&[0b1011_0000, 0b0100_1000]);
    let pdf_mono = build_image_pdf(5, 2, "/DeviceGray", 1, Some("/FlateDecode"), "", &mono_packed);
    let p_mono = renderer.render_page(&pdf_mono, 1, DEFAULT_RENDER_DPI).unwrap();
    assert_eq!((p_mono.width, p_mono.height, p_mono.source_kind), (5, 2, RenderedPageKind::ExtractedRasterXObject));
    assert_eq!(&p_mono.png_bytes[..8], PNG_SIGNATURE);
    assert_eq!(p_mono.sha256, ledger::sha256_hex(&p_mono.png_bytes));
    assert_eq!(decode_png(&p_mono.png_bytes, 64, 64, 4096).unwrap().pixels, vec![255, 0, 255, 255, 0, 0, 255, 0, 0, 255]);

    let pdf_inv = build_image_pdf(5, 2, "/DeviceGray", 1, Some("/FlateDecode"), "/Decode [1 0]", &mono_packed);
    let p_inv = renderer.render_page(&pdf_inv, 1, DEFAULT_RENDER_DPI).unwrap();
    assert_eq!(decode_png(&p_inv.png_bytes, 64, 64, 4096).unwrap().pixels, vec![0, 255, 0, 0, 255, 255, 0, 255, 255, 0]);

    // 2) 8-bit Grayscale multi-page PDF.
    let mp_pdf = build_multipage_gray_pdf(&[(2, 2, &[10, 20, 30, 40]), (3, 1, &[100, 150, 200])]);
    let mp1 = renderer.render_page(&mp_pdf, 1, 150).unwrap();
    let mp2 = renderer.render_page(&mp_pdf, 2, 150).unwrap();
    assert_eq!(decode_png(&mp1.png_bytes, 64, 64, 4096).unwrap().pixels, vec![10, 20, 30, 40]);
    assert_eq!(decode_png(&mp2.png_bytes, 64, 64, 4096).unwrap().pixels, vec![100, 150, 200]);

    // 3) 8-bit RGB with FlateDecode + PNG Up predictor (/Predictor 12).
    let rgb_predicted = zlib_compress(&[2, 10, 20, 30, 40, 50, 60, 2, 5, 5, 5, 10, 10, 10]);
    let pdf_rgb = build_image_pdf(
        2, 2, "/DeviceRGB", 8, Some("[/FlateDecode]"),
        "/DecodeParms << /Predictor 12 /Colors 3 /BitsPerComponent 8 /Columns 2 >>",
        &rgb_predicted,
    );
    let p_rgb = renderer.render_page(&pdf_rgb, 1, 200).unwrap();
    assert_eq!(
        decode_png(&p_rgb.png_bytes, 64, 64, 4096).unwrap().pixels,
        vec![10, 20, 30, 40, 50, 60, 15, 25, 35, 50, 60, 70]
    );

    // 4) 8-bit CMYK (standard + inverted /Decode [1 0 1 0 1 0 1 0]).
    let cmyk_raw = zlib_compress(&[0, 0, 0, 0, 0, 0, 0, 255, 0, 255, 255, 0, 128, 0, 0, 128]);
    let pdf_cmyk = build_image_pdf(2, 2, "/DeviceCMYK", 8, Some("/FlateDecode"), "", &cmyk_raw);
    let p_cmyk = renderer.render_page(&pdf_cmyk, 1, 200).unwrap();
    assert_eq!(
        decode_png(&p_cmyk.png_bytes, 64, 64, 4096).unwrap().pixels,
        vec![255, 255, 255, 0, 0, 0, 255, 0, 0, 63, 127, 127]
    );
    let cmyk_inv = zlib_compress(&[255, 0, 0, 255]);
    let pdf_cmyk_inv = build_image_pdf(1, 1, "/DeviceCMYK", 8, Some("/FlateDecode"), "/Decode [1 0 1 0 1 0 1 0]", &cmyk_inv);
    let p_cmyk_inv = renderer.render_page(&pdf_cmyk_inv, 1, 200).unwrap();
    assert_eq!(decode_png(&p_cmyk_inv.png_bytes, 64, 64, 4096).unwrap().pixels, vec![255, 0, 0]);

    // 5) 4-bit /Indexed /DeviceRGB palette.
    let idx_data = zlib_compress(&[0x01, 0x20, 0x32, 0x10]);
    let pdf_idx = build_image_pdf(
        3, 2, "[/Indexed /DeviceRGB 3 <FF000000FF000000FFFFFF00>]", 4, Some("/FlateDecode"), "", &idx_data,
    );
    let p_idx = renderer.render_page(&pdf_idx, 1, 200).unwrap();
    assert_eq!(
        decode_png(&p_idx.png_bytes, 64, 64, 4096).unwrap().pixels,
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 0, 255, 0]
    );
}

#[test]
fn raster_renderer_enforces_bounds_and_rejects_malformed_or_unsupported_inputs() {
    let renderer = RasterXObjectRenderer::default();
    let valid_pdf = build_multipage_gray_pdf(&[(2, 2, &[1, 2, 3, 4])]);

    assert!(renderer.render_page(&valid_pdf, 0, 200).unwrap_err().contains(">= 1"));
    assert!(renderer.render_page(&valid_pdf, 2, 200).unwrap_err().contains("out of range"));
    assert!(renderer.render_page(&valid_pdf, 1, 71).unwrap_err().contains("DPI"));
    assert!(renderer.render_page(&valid_pdf, 1, 601).unwrap_err().contains("DPI"));
    assert!(renderer.render_page(b"", 1, 200).unwrap_err().contains("empty"));
    assert!(renderer.render_page(b"not a pdf at all", 1, 200).unwrap_err().contains("%PDF-"));
    assert!(renderer.render_page(b"%PDF-1.4\ngarbage_truncated", 1, 200).is_err());

    let tiny_cap = RasterXObjectRenderer::new(RenderLimits { max_pdf_bytes: 32, ..RenderLimits::default() });
    assert!(tiny_cap.render_page(&valid_pdf, 1, 200).unwrap_err().contains("exceeds render limit"));

    let bomb = build_oversized_dim_pdf(20_000, 20_000);
    assert!(renderer.render_page(&bomb, 1, 200).unwrap_err().contains("exceed limit"));

    let enc = build_encrypted_pdf();
    assert!(renderer.render_page(&enc, 1, 200).unwrap_err().contains("encrypted"));

    let dct = build_image_pdf(2, 2, "/DeviceRGB", 8, Some("/DCTDecode"), "", &[0xFF, 0xD8, 0xFF, 0xD9]);
    assert!(renderer.render_page(&dct, 1, 200).unwrap_err().contains("DCTDecode"));

    let vec_pdf = build_vector_text_pdf("Vector text only");
    assert!(renderer.render_page(&vec_pdf, 1, 200).unwrap_err().contains("no raster image XObject found on page 1"));

    let missing_helper = ExternalCommandRenderer::pdftoppm(std::env::temp_dir().join("nonexistent-pdftoppm-bin"), RenderLimits::default());
    assert!(missing_helper.render_page(&valid_pdf, 1, 200).unwrap_err().contains("not found"));
}

#[test]
fn map_renderer_fallback_delegation_and_render_crop() {
    let vec_pdf = build_vector_text_pdf("Needs fallback");
    let scan_pdf = build_multipage_gray_pdf(&[(4, 4, &(0..16u8).collect::<Vec<_>>())]);
    let injected_png = encode_rgb_png(4, 4, &[42u8; 4 * 4 * 3]).unwrap();

    let mut pages = BTreeMap::new();
    pages.insert(1, injected_png.clone());
    let map = MapPageRenderer::new(pages.clone());
    assert_eq!(map.renderer_id(), "pdf-map-injected/1");

    let inj = map.render_page(&vec_pdf, 1, 200).unwrap();
    assert_eq!((inj.width, inj.height, inj.source_kind), (4, 4, RenderedPageKind::Injected));
    assert_eq!(inj.sha256, ledger::sha256_hex(&injected_png));
    assert!(map.render_page(&vec_pdf, 2, 200).unwrap_err().contains("no injected PNG"));

    // Fallback delegates when primary fails on vector-only PDF, and uses primary when raster exists.
    let fb = FallbackPageRenderer::new(
        Box::new(RasterXObjectRenderer::default()),
        Some(Box::new(MapPageRenderer::new(pages))),
    );
    let via_primary = fb.render_page(&scan_pdf, 1, 200).unwrap();
    assert_eq!(via_primary.source_kind, RenderedPageKind::ExtractedRasterXObject);

    let via_fallback = fb.render_page(&vec_pdf, 1, 200).unwrap();
    assert_eq!(via_fallback.source_kind, RenderedPageKind::Injected);

    let cropped = fb.render_crop(&scan_pdf, 1, 200, CropRect::new(0.5, 0.5, 0.5, 0.5).unwrap()).unwrap();
    assert_eq!((cropped.width, cropped.height, cropped.source_kind), (2, 2, RenderedPageKind::ExtractedRasterXObject));
    assert_eq!(cropped.sha256, ledger::sha256_hex(&cropped.png_bytes));
    assert_eq!(decode_png(&cropped.png_bytes, 16, 16, 256).unwrap().pixels, vec![10, 11, 14, 15]);
}

#[test]
fn render_crop_enforces_renderer_limits() {
    let scan_pdf = build_multipage_gray_pdf(&[(8, 8, &(0..64u8).collect::<Vec<_>>())]);
    let full = RasterXObjectRenderer::default().render_page(&scan_pdf, 1, 200).unwrap();
    let tight = RenderLimits {
        max_png_bytes: full.png_bytes.len() + 16,
        max_width: 8,
        max_height: 8,
        max_pixels: 64,
        ..RenderLimits::default()
    };
    let mut pages = BTreeMap::new();
    pages.insert(1, full.png_bytes);
    let constrained = MapPageRenderer::new(pages).with_limits(tight);
    assert_eq!(constrained.render_limits(), tight);
    assert!(constrained.render_crop(&scan_pdf, 1, 200, CropRect::new(0.0, 0.0, 0.5, 0.5).unwrap()).is_ok());

    let tiny_out = RenderLimits { max_png_bytes: 16, ..tight };
    let raster_tiny = RasterXObjectRenderer::new(tiny_out);
    assert!(raster_tiny.render_crop(&scan_pdf, 1, 200, CropRect::new(0.0, 0.0, 0.5, 0.5).unwrap()).is_err());
}

