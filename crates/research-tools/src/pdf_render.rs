//! Pure-Rust PDF page rendering (raster XObject extraction, opt-in external helper,
//! fallback dispatch, and injectable test renderer) for package F14b.

use crate::pdf_render_png::{
    cmyk_to_rgb, crop_png, encode_png, indexed_rgb_to_rgb, inflate_zlib_bounded,
    inspect_png_header, reverse_png_predictor, unpack_1bit_gray, ColorType, RawImage,
};
use pdf_extract::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const DEFAULT_RENDER_DPI: u16 = 200;
pub const MIN_RENDER_DPI: u16 = 72;
pub const MAX_RENDER_DPI: u16 = 600;
pub const MAX_RENDER_WIDTH: u32 = 8192;
pub const MAX_RENDER_HEIGHT: u32 = 8192;
pub const MAX_RENDER_PIXELS: u64 = 36_000_000;
pub const MAX_RENDER_PNG_BYTES: usize = 32 * 1024 * 1024;
pub const DEFAULT_RENDER_TIMEOUT_MS: u64 = 15_000;
pub const MAX_RENDER_PDF_BYTES: usize = crate::pdf_pages::MAX_USER_PDF_BYTES;

const MAX_TREE_DEPTH: usize = 16;
const MAX_REF_DEPTH: usize = 8;
static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Provenance of a rendered PDF page PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderedPageKind {
    ExtractedRasterXObject,
    ExternalHelper,
    Injected,
}

/// Rendered PNG image and integrity metadata for one 1-based physical PDF page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub page: usize,
    pub width: u32,
    pub height: u32,
    pub dpi: u16,
    pub png_bytes: Vec<u8>,
    pub sha256: String,
    pub source_kind: RenderedPageKind,
}

impl RenderedPage {
    fn from_png(page: usize, width: u32, height: u32, dpi: u16, png_bytes: Vec<u8>, source_kind: RenderedPageKind) -> Self {
        let sha256 = ledger::sha256_hex(&png_bytes);
        Self { page, width, height, dpi, png_bytes, sha256, source_kind }
    }
}

/// Normalized `[0, 1]` crop rectangle with top-left origin `(x, y)` and size `(w, h)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl CropRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Result<Self, String> {
        if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() { return Err("crop coordinates must be finite numbers".into()); }
        if x < 0.0 || y < 0.0 || x >= 1.0 || y >= 1.0 { return Err(format!("crop origin ({x}, {y}) must satisfy 0 <= x,y < 1")); }
        if w <= 0.0 || h <= 0.0 { return Err(format!("crop dimensions ({w}, {h}) must be > 0")); }
        if x + w > 1.001 || y + h > 1.001 { return Err(format!("crop box ({x}, {y}, {w}, {h}) exceeds unit page bounds")); }
        let (cw, ch) = (w.min(1.0 - x), h.min(1.0 - y));
        if cw <= 0.0 || ch <= 0.0 { return Err("crop rectangle collapses to zero area".into()); }
        Ok(Self { x, y, w: cw, h: ch })
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 4 { return Err("crop must be formatted as `x,y,w,h`".into()); }
        let p = |raw: &str, name: &str| raw.trim().parse::<f64>().map_err(|_| format!("invalid crop {name}: `{}`", raw.trim()));
        Self::new(p(parts[0], "x")?, p(parts[1], "y")?, p(parts[2], "w")?, p(parts[3], "h")?)
    }

    pub(crate) fn validate(&self) -> Result<Self, String> { Self::new(self.x, self.y, self.w, self.h) }
}

/// Resource bounds enforced across all PDF page renderers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderLimits {
    pub max_pdf_bytes: usize,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_png_bytes: usize,
    pub timeout_ms: u64,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_pdf_bytes: MAX_RENDER_PDF_BYTES,
            max_width: MAX_RENDER_WIDTH,
            max_height: MAX_RENDER_HEIGHT,
            max_pixels: MAX_RENDER_PIXELS,
            max_png_bytes: MAX_RENDER_PNG_BYTES,
            timeout_ms: DEFAULT_RENDER_TIMEOUT_MS,
        }
    }
}

/// Pluggable PDF page-to-PNG renderer interface.
pub trait PdfPageRenderer: Send + Sync {
    fn renderer_id(&self) -> &str;
    fn render_limits(&self) -> RenderLimits { RenderLimits::default() }
    fn render_page(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16) -> Result<RenderedPage, String>;

    fn render_crop(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16, crop: CropRect) -> Result<RenderedPage, String> {
        let crop = crop.validate()?;
        let full = self.render_page(pdf_bytes, page_1based, dpi)?;
        let (png, w, h) = crop_png(&full.png_bytes, &crop, &self.render_limits())?;
        Ok(RenderedPage::from_png(full.page, w, h, full.dpi, png, full.source_kind))
    }
}

/// Encode an 8-bit grayscale buffer into a valid PNG byte stream.
pub fn encode_grayscale_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    encode_png(&RawImage { width, height, color_type: ColorType::Gray8, pixels: pixels.to_vec() }, MAX_RENDER_PNG_BYTES)
}

/// Encode an 8-bit RGB buffer into a valid PNG byte stream.
pub fn encode_rgb_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    encode_png(&RawImage { width, height, color_type: ColorType::Rgb8, pixels: pixels.to_vec() }, MAX_RENDER_PNG_BYTES)
}

/// Read `(width, height)` from a PNG header without decompressing pixel data.
pub fn inspect_png_dimensions(png_bytes: &[u8]) -> Result<(u32, u32), String> { inspect_png_header(png_bytes) }

fn validate_request(pdf: &[u8], page: usize, dpi: u16, limits: &RenderLimits) -> Result<(), String> {
    if page == 0 { return Err("page number must be >= 1".into()); }
    if !(MIN_RENDER_DPI..=MAX_RENDER_DPI).contains(&dpi) { return Err(format!("render DPI {dpi} out of bounds ({MIN_RENDER_DPI}..={MAX_RENDER_DPI})")); }
    if pdf.is_empty() { return Err("empty PDF input".into()); }
    if pdf.len() > limits.max_pdf_bytes { return Err(format!("PDF size ({}) exceeds render limit ({})", pdf.len(), limits.max_pdf_bytes)); }
    if !pdf[..pdf.len().min(1024)].windows(5).any(|w| w == b"%PDF-") { return Err("missing %PDF- header".into()); }
    Ok(())
}

fn check_dims(width: u64, height: u64, limits: &RenderLimits) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 { return Err("rendered image has zero width or height".into()); }
    if width > u64::from(limits.max_width) || height > u64::from(limits.max_height) {
        return Err(format!("image dimensions {width}x{height} exceed limit {}x{}", limits.max_width, limits.max_height));
    }
    let px = width.saturating_mul(height);
    if px > limits.max_pixels { return Err(format!("image pixel count ({px}) exceeds limit ({})", limits.max_pixels)); }
    Ok((u32::try_from(width).map_err(|_| "image width overflow")?, u32::try_from(height).map_err(|_| "image height overflow")?))
}

fn validate_png_against_limits(png: &[u8], limits: &RenderLimits) -> Result<(u32, u32), String> {
    if png.len() > limits.max_png_bytes {
        return Err(format!("PNG ({} bytes) exceeds limit ({} bytes)", png.len(), limits.max_png_bytes));
    }
    let (w, h) = inspect_png_header(png)?;
    check_dims(u64::from(w), u64::from(h), limits)
}

fn resolve_obj<'a>(doc: &'a Document, mut obj: &'a Object) -> Option<&'a Object> {
    for _ in 0..MAX_REF_DEPTH {
        match obj { Object::Reference(id) => obj = doc.get_object(*id).ok()?, other => return Some(other) }
    }
    None
}

fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match resolve_obj(doc, obj)? { Object::Dictionary(d) => Some(d), Object::Stream(s) => Some(&s.dict), _ => None }
}

fn inherited_attr<'a>(doc: &'a Document, mut cur: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_TREE_DEPTH {
        if !seen.insert(cur) { break; }
        let dict = doc.get_object(cur).ok()?.as_dict().ok()?;
        if let Ok(val) = dict.get(key) { return Some(val); }
        match dict.get(b"Parent").ok()? { Object::Reference(parent) => cur = *parent, _ => break }
    }
    None
}

fn obj_i64(doc: &Document, obj: &Object) -> Option<i64> {
    match resolve_obj(doc, obj)? { Object::Integer(v) => Some(*v), _ => None }
}

fn dict_i64(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    dict.get(key).ok().and_then(|o| obj_i64(doc, o))
}

fn obj_f64(doc: &Document, obj: &Object) -> Option<f64> {
    match resolve_obj(doc, obj)? { Object::Integer(v) => Some(*v as f64), Object::Real(v) => Some(f64::from(*v)), _ => None }
}

fn collect_image_xobjects<'a>(doc: &'a Document, resources: &'a Dictionary, depth: usize, out: &mut Vec<(u64, u64, &'a Stream)>) {
    let Some(xobjs) = resources.get(b"XObject").ok().and_then(|o| resolve_dict(doc, o)) else { return; };
    for (_, val) in xobjs.iter() {
        let Some(Object::Stream(stream)) = resolve_obj(doc, val) else { continue; };
        let sub = stream.dict.get(b"Subtype").ok().and_then(|o| resolve_obj(doc, o)).and_then(|o| o.as_name().ok());
        match sub {
            Some(b"Image") => {
                if let (Some(w), Some(h)) = (dict_i64(doc, &stream.dict, b"Width"), dict_i64(doc, &stream.dict, b"Height")) {
                    if w > 0 && h > 0 { out.push((w as u64, h as u64, stream)); }
                }
            }
            Some(b"Form") if depth < 1 => {
                if let Some(res) = stream.dict.get(b"Resources").ok().and_then(|o| resolve_dict(doc, o)) {
                    collect_image_xobjects(doc, res, depth + 1, out);
                }
            }
            _ => {}
        }
    }
}

enum PdfColorSpace {
    Gray,
    Rgb,
    Cmyk,
    IndexedRgb { hival: u8, palette: Vec<u8> },
}

fn parse_filter(doc: &Document, dict: &Dictionary, page: usize) -> Result<bool, String> {
    let Ok(raw) = dict.get(b"Filter") else { return Ok(false); };
    let resolved = resolve_obj(doc, raw).ok_or_else(|| format!("unresolvable /Filter on page {page}"))?;
    let check = |name: &[u8]| match name {
        b"FlateDecode" | b"Fl" => Ok(true),
        other => Err(format!("unsupported PDF image filter `{}` on page {page}", String::from_utf8_lossy(other))),
    };
    match resolved {
        Object::Name(n) => check(n),
        Object::Array(a) if a.is_empty() => Ok(false),
        Object::Array(a) if a.len() == 1 => match resolve_obj(doc, &a[0]) {
            Some(Object::Name(n)) => check(n),
            _ => Err(format!("malformed /Filter array entry on page {page}")),
        },
        Object::Array(a) => {
            let names: Vec<String> = a.iter().map(|it| match resolve_obj(doc, it) {
                Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
                _ => "unknown".into(),
            }).collect();
            Err(format!("unsupported PDF image filter chain `[{}]` on page {page}", names.join(", ")))
        }
        _ => Err(format!("malformed /Filter value on page {page}")),
    }
}

fn parse_color_space(doc: &Document, dict: &Dictionary, page: usize) -> Result<PdfColorSpace, String> {
    let Ok(raw) = dict.get(b"ColorSpace") else {
        let mask = dict.get(b"ImageMask").ok().and_then(|o| resolve_obj(doc, o)).is_some_and(|o| matches!(o, Object::Boolean(true)));
        return if mask { Ok(PdfColorSpace::Gray) } else { Err(format!("PDF image missing /ColorSpace on page {page}")) };
    };
    let cs = resolve_obj(doc, raw).ok_or_else(|| format!("unresolvable /ColorSpace on page {page}"))?;
    let named = |n: &[u8]| match n {
        b"DeviceGray" | b"G" => Some(PdfColorSpace::Gray),
        b"DeviceRGB" | b"RGB" => Some(PdfColorSpace::Rgb),
        b"DeviceCMYK" | b"CMYK" => Some(PdfColorSpace::Cmyk),
        _ => None,
    };
    match cs {
        Object::Name(n) => named(n).ok_or_else(|| format!("unsupported /ColorSpace `{}` on page {page}", String::from_utf8_lossy(n))),
        Object::Array(a) if a.len() == 1 => match resolve_obj(doc, &a[0]) {
            Some(Object::Name(n)) => named(n).ok_or_else(|| format!("unsupported /ColorSpace array on page {page}")),
            _ => Err(format!("unsupported /ColorSpace array on page {page}")),
        },
        Object::Array(a) if a.len() >= 4 => {
            let is_idx = matches!(resolve_obj(doc, &a[0]), Some(Object::Name(n)) if n.as_slice() == b"Indexed" || n.as_slice() == b"I");
            let is_rgb = match resolve_obj(doc, &a[1]) {
                Some(Object::Name(n)) => n.as_slice() == b"DeviceRGB" || n.as_slice() == b"RGB",
                Some(Object::Array(b)) if b.len() == 1 => matches!(resolve_obj(doc, &b[0]), Some(Object::Name(n)) if n.as_slice() == b"DeviceRGB" || n.as_slice() == b"RGB"),
                _ => false,
            };
            if !is_idx || !is_rgb { return Err(format!("only /Indexed /DeviceRGB palettes are supported on page {page}")); }
            let hi = obj_i64(doc, &a[2]).ok_or_else(|| format!("invalid /Indexed hival on page {page}"))?;
            if !(0..=255).contains(&hi) { return Err(format!("/Indexed hival {hi} out of 0..=255 range")); }
            let hival = hi as u8;
            let needed = (usize::from(hival) + 1) * 3;
            let palette = match resolve_obj(doc, &a[3]) {
                Some(Object::String(b, _)) => b.clone(),
                Some(Object::Stream(ps)) => if parse_filter(doc, &ps.dict, page)? { inflate_zlib_bounded(&ps.content, 256 * 3)? } else { ps.content.clone() },
                _ => return Err(format!("unsupported /Indexed lookup table on page {page}")),
            };
            if palette.len() < needed { return Err(format!("/Indexed palette too short ({}, need {needed})", palette.len())); }
            Ok(PdfColorSpace::IndexedRgb { hival, palette })
        }
        _ => Err(format!("unsupported PDF image /ColorSpace on page {page}")),
    }
}

fn decode_stream_bytes(stream: &Stream, is_flate: bool, expected: usize) -> Result<Vec<u8>, String> {
    if is_flate {
        let buf = inflate_zlib_bounded(&stream.content, expected)?;
        if buf.len() < expected { return Err("truncated FlateDecode PDF image stream".into()); }
        Ok(buf)
    } else if stream.content.len() < expected {
        Err("truncated uncompressed PDF image stream".into())
    } else {
        Ok(stream.content[..expected].to_vec())
    }
}

fn decode_xobject_to_image(doc: &Document, stream: &Stream, width: u32, height: u32, page: usize) -> Result<RawImage, String> {
    let is_flate = parse_filter(doc, &stream.dict, page)?;
    let cs = parse_color_space(doc, &stream.dict, page)?;
    let bpc_i = dict_i64(doc, &stream.dict, b"BitsPerComponent").unwrap_or(1);
    let bpc = u8::try_from(bpc_i).map_err(|_| format!("invalid /BitsPerComponent {bpc_i}"))?;
    let comps: usize = match (&cs, bpc) {
        (PdfColorSpace::Gray, 1 | 8) | (PdfColorSpace::IndexedRgb { .. }, 1 | 4 | 8) => 1,
        (PdfColorSpace::Rgb, 8) => 3,
        (PdfColorSpace::Cmyk, 8) => 4,
        _ => return Err(format!("unsupported BitsPerComponent={bpc} for color space on page {page}")),
    };

    let (w, h) = (width as usize, height as usize);
    let row_bytes = w.checked_mul(comps).and_then(|v| v.checked_mul(usize::from(bpc))).ok_or("row bit overflow")?.div_ceil(8);
    let expected_samples = row_bytes.checked_mul(h).ok_or("sample buffer overflow")?;
    let parms = stream.dict.get(b"DecodeParms").ok().and_then(|o| match resolve_obj(doc, o)? {
        Object::Dictionary(d) => Some(d),
        Object::Array(a) if !a.is_empty() => resolve_dict(doc, &a[0]),
        _ => None,
    });
    let predictor = parms.and_then(|d| dict_i64(doc, d, b"Predictor")).unwrap_or(1);
    let samples = match predictor {
        1 => decode_stream_bytes(stream, is_flate, expected_samples)?,
        10..=15 => {
            let p_cols = parms.and_then(|d| dict_i64(doc, d, b"Columns")).and_then(|v| usize::try_from(v).ok()).unwrap_or(w);
            let p_c = parms.and_then(|d| dict_i64(doc, d, b"Colors")).and_then(|v| usize::try_from(v).ok()).unwrap_or(comps);
            let p_b = parms.and_then(|d| dict_i64(doc, d, b"BitsPerComponent")).and_then(|v| usize::try_from(v).ok()).unwrap_or(usize::from(bpc));
            let p_row = p_cols.checked_mul(p_c).and_then(|v| v.checked_mul(p_b)).ok_or("predictor row overflow")?.div_ceil(8);
            let p_bpp = p_c.checked_mul(p_b).ok_or("predictor bpp overflow")?.div_ceil(8).max(1);
            if p_row != row_bytes || p_row % p_bpp != 0 { return Err("PDF PNG predictor geometry mismatch".into()); }
            let expected_raw = p_row.checked_add(1).and_then(|s| s.checked_mul(h)).ok_or("predictor stream overflow")?;
            let raw = decode_stream_bytes(stream, is_flate, expected_raw)?;
            reverse_png_predictor(&raw, p_row / p_bpp, h, p_bpp)?
        }
        other => return Err(format!("unsupported PDF /Predictor {other} on page {page}")),
    };

    let dec: Vec<f64> = stream.dict.get(b"Decode").ok().and_then(|o| resolve_obj(doc, o))
        .and_then(|o| o.as_array().ok()).map(|a| a.iter().filter_map(|v| obj_f64(doc, v)).collect()).unwrap_or_default();
    match cs {
        PdfColorSpace::Gray => {
            let inv = dec.len() >= 2 && dec[0] > dec[1];
            let pixels = if bpc == 1 { unpack_1bit_gray(&samples, w, h, row_bytes, inv)? }
                else if inv { samples.into_iter().map(|v| 255 - v).collect() } else { samples };
            Ok(RawImage { width, height, color_type: ColorType::Gray8, pixels })
        }
        PdfColorSpace::Rgb => Ok(RawImage { width, height, color_type: ColorType::Rgb8, pixels: samples }),
        PdfColorSpace::Cmyk => {
            let inv = dec.len() >= 8 && dec[0] > dec[1] && dec[2] > dec[3] && dec[4] > dec[5] && dec[6] > dec[7];
            Ok(RawImage { width, height, color_type: ColorType::Rgb8, pixels: cmyk_to_rgb(&samples, w * h, inv) })
        }
        PdfColorSpace::IndexedRgb { hival, palette } => Ok(RawImage {
            width, height, color_type: ColorType::Rgb8,
            pixels: indexed_rgb_to_rgb(&samples, w, h, row_bytes, bpc, hival, &palette),
        }),
    }
}

/// Pure-Rust renderer that extracts the largest raster Image XObject on a PDF page.
#[derive(Debug, Clone, Default)]
pub struct RasterXObjectRenderer {
    pub limits: RenderLimits,
}

impl RasterXObjectRenderer {
    pub fn new(limits: RenderLimits) -> Self { Self { limits } }
}

impl PdfPageRenderer for RasterXObjectRenderer {
    fn renderer_id(&self) -> &str { "pdf-raster-xobject/1" }
    fn render_limits(&self) -> RenderLimits { self.limits }

    fn render_page(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16) -> Result<RenderedPage, String> {
        validate_request(pdf_bytes, page_1based, dpi, &self.limits)?;
        let limits = self.limits;
        catch_unwind(AssertUnwindSafe(|| {
            let mut doc = Document::load_mem(pdf_bytes).map_err(|e| format!("malformed PDF structure: {e}"))?;
            if doc.is_encrypted() && doc.decrypt("").is_err() {
                return Err("encrypted password-protected PDFs are not supported".into());
            }
            let pages = doc.get_pages();
            let page_u32 = u32::try_from(page_1based).map_err(|_| format!("page {page_1based} out of range"))?;
            let &page_id = pages.get(&page_u32).ok_or_else(|| {
                format!("page {page_1based} out of range (document has {} page(s))", pages.len())
            })?;
            let Some(res) = inherited_attr(&doc, page_id, b"Resources").and_then(|o| resolve_dict(&doc, o)) else {
                return Err(format!("no raster image XObject found on page {page_1based}"));
            };
            let mut images = Vec::new();
            collect_image_xobjects(&doc, res, 0, &mut images);
            let Some(&(rw, rh, stream)) = images.iter().max_by_key(|&&(w, h, _)| u128::from(w) * u128::from(h)) else {
                return Err(format!("no raster image XObject found on page {page_1based}"));
            };
            let (w, h) = check_dims(rw, rh, &limits)?;
            let raw_img = decode_xobject_to_image(&doc, stream, w, h, page_1based)?;
            let png = encode_png(&raw_img, limits.max_png_bytes)?;
            Ok(RenderedPage::from_png(page_1based, w, h, dpi, png, RenderedPageKind::ExtractedRasterXObject))
        }))
        .unwrap_or_else(|_| Err("PDF parser trapped an internal panic".into()))
    }
}

struct TempDirGuard(PathBuf);
impl TempDirGuard {
    fn create() -> Result<Self, String> {
        let (base, pid) = (std::env::temp_dir(), std::process::id());
        for _ in 0..32 {
            let dir = base.join(format!("research-pdf-render-{pid}-{}", TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed)));
            if fs::create_dir(&dir).is_ok() { return Ok(Self(dir)); }
        }
        Err("failed to create temporary directory for PDF rendering".into())
    }
}
impl Drop for TempDirGuard {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

/// Strictly opt-in external helper renderer (such as `pdftoppm`) with bounded resources.
#[derive(Debug, Clone)]
pub struct ExternalCommandRenderer {
    pub executable: PathBuf,
    pub limits: RenderLimits,
    pub id: String,
}
impl ExternalCommandRenderer {
    pub fn pdftoppm(executable: impl Into<PathBuf>, limits: RenderLimits) -> Self {
        Self { executable: executable.into(), limits, id: "external-pdftoppm/1".into() }
    }
}
impl PdfPageRenderer for ExternalCommandRenderer {
    fn renderer_id(&self) -> &str { &self.id }
    fn render_limits(&self) -> RenderLimits { self.limits }
    fn render_page(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16) -> Result<RenderedPage, String> {
        validate_request(pdf_bytes, page_1based, dpi, &self.limits)?;
        if !self.executable.is_file() { return Err(format!("external PDF renderer not found: {}", self.executable.display())); }
        let temp = TempDirGuard::create()?;
        fs::write(temp.0.join("input.pdf"), pdf_bytes).map_err(|e| format!("failed writing temp PDF: {e}"))?;
        let mut cmd = Command::new(&self.executable);
        cmd.env_clear().current_dir(&temp.0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(path_env) = std::env::var_os("PATH") { cmd.env("PATH", path_env); }
        #[cfg(windows)]
        {
            if let Some(sysroot) = std::env::var_os("SYSTEMROOT") { cmd.env("SYSTEMROOT", sysroot); }
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let p_str = page_1based.to_string();
        cmd.args(["-png", "-r", &dpi.to_string(), "-f", &p_str, "-l", &p_str, "-singlefile", "input.pdf", "page"]);
        let mut child = cmd.spawn().map_err(|e| format!("failed spawning external renderer: {e}"))?;
        let deadline = Instant::now() + Duration::from_millis(self.limits.timeout_ms.max(1));
        let status = loop {
            match child.try_wait().map_err(|e| format!("failed polling external renderer: {e}"))? {
                Some(st) => break st,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("external PDF renderer timed out after {} ms", self.limits.timeout_ms));
                }
                None => std::thread::sleep(Duration::from_millis(15)),
            }
        };
        if !status.success() { return Err(format!("external PDF renderer exited with status {status}")); }
        let png_path = find_rendered_png(&temp.0)?;
        let cap = u64::try_from(self.limits.max_png_bytes).ok().and_then(|n| n.checked_add(1)).ok_or("PNG cap overflow")?;
        let mut png = Vec::new();
        File::open(&png_path).map_err(|e| format!("failed opening rendered PNG: {e}"))?
            .take(cap).read_to_end(&mut png).map_err(|e| format!("failed reading rendered PNG: {e}"))?;
        let (w, h) = validate_png_against_limits(&png, &self.limits)?;
        Ok(RenderedPage::from_png(page_1based, w, h, dpi, png, RenderedPageKind::ExternalHelper))
    }
}

fn find_rendered_png(dir: &Path) -> Result<PathBuf, String> {
    let direct = dir.join("page.png");
    if direct.is_file() { return Ok(direct); }
    for entry in fs::read_dir(dir).map_err(|e| format!("failed reading temp render dir: {e}"))?.flatten() {
        let p = entry.path();
        if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("page") && n.ends_with(".png")) && p.is_file() {
            return Ok(p);
        }
    }
    Err("external PDF renderer produced no PNG output".into())
}

/// Delegates to `fallback` whenever `primary` returns an error.
pub struct FallbackPageRenderer {
    pub primary: Box<dyn PdfPageRenderer>,
    pub fallback: Option<Box<dyn PdfPageRenderer>>,
}
impl FallbackPageRenderer {
    pub fn new(primary: Box<dyn PdfPageRenderer>, fallback: Option<Box<dyn PdfPageRenderer>>) -> Self { Self { primary, fallback } }
}
impl PdfPageRenderer for FallbackPageRenderer {
    fn renderer_id(&self) -> &str { self.primary.renderer_id() }
    fn render_limits(&self) -> RenderLimits { self.primary.render_limits() }
    fn render_page(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16) -> Result<RenderedPage, String> {
        self.primary.render_page(pdf_bytes, page_1based, dpi).or_else(|pe| match &self.fallback {
            Some(fb) => fb.render_page(pdf_bytes, page_1based, dpi).map_err(|fe| format!("primary renderer failed ({pe}); fallback failed ({fe})")),
            None => Err(pe),
        })
    }
}

/// Injectable in-memory PNG map renderer for deterministic tests.
#[derive(Debug, Clone, Default)]
pub struct MapPageRenderer {
    pub pages: BTreeMap<usize, Vec<u8>>,
    pub limits: RenderLimits,
}
impl MapPageRenderer {
    pub fn new(pages: BTreeMap<usize, Vec<u8>>) -> Self { Self { pages, limits: RenderLimits::default() } }
    pub fn with_limits(mut self, limits: RenderLimits) -> Self { self.limits = limits; self }
}
impl PdfPageRenderer for MapPageRenderer {
    fn renderer_id(&self) -> &str { "pdf-map-injected/1" }
    fn render_limits(&self) -> RenderLimits { self.limits }
    fn render_page(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16) -> Result<RenderedPage, String> {
        validate_request(pdf_bytes, page_1based, dpi, &self.limits)?;
        let png = self.pages.get(&page_1based).ok_or_else(|| format!("no injected PNG for page {page_1based}"))?;
        let (w, h) = validate_png_against_limits(png, &self.limits)?;
        Ok(RenderedPage::from_png(page_1based, w, h, dpi, png.clone(), RenderedPageKind::Injected))
    }
}

#[cfg(test)]
#[path = "pdf_render_tests.rs"]
mod tests;
