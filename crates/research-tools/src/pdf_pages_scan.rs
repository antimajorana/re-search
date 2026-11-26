//! PDF page-stream scan-detection signals (`MediaBox`, Image XObject coverage, `3 Tr`
//! invisible OCR text rendering mode) and [`PageBasis`] classification (package F14a).

use crate::pdf_pages::{PageBasis, PageQualityMetrics};
use pdf_extract::content::Content;
use pdf_extract::{Dictionary, Document, Object, ObjectId};
use std::collections::BTreeSet;

const MAX_CONTENT_OPS: usize = 200_000;
const MAX_TREE_DEPTH: usize = 16;
const STANDARD_PAGE_AREA_PT2: f64 = 612.0 * 792.0;

#[derive(Debug, Clone, Copy)]
struct Affine2d {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine2d {
    fn identity() -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }

    fn concat(self, m: Self) -> Self {
        // PDF `cm` pre-multiplies M onto CTM: CTM' = M * CTM.
        Self {
            a: m.a * self.a + m.b * self.c,
            b: m.a * self.b + m.b * self.d,
            c: m.c * self.a + m.d * self.c,
            d: m.c * self.b + m.d * self.d,
            e: m.e * self.a + m.f * self.c + self.e,
            f: m.e * self.b + m.f * self.d + self.f,
        }
    }

    fn unit_square_area(self) -> f64 {
        let det = (self.a * self.d - self.b * self.c).abs();
        if det.is_finite() { det } else { 0.0 }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PageStreamSignals {
    pub width_pt: f64,
    pub height_pt: f64,
    pub max_image_coverage: f64,
    pub total_image_coverage: f64,
    pub has_inline_image: bool,
    pub total_text_ops: usize,
    pub invisible_text_ops: usize,
}

impl PageStreamSignals {
    pub fn has_invisible_text(&self) -> bool {
        self.total_text_ops > 0 && self.invisible_text_ops * 2 >= self.total_text_ops
    }
}

fn obj_as_f64(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Dictionary(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        },
        _ => None,
    }
}

fn inherited_page_attr<'a>(
    doc: &'a Document,
    page_id: ObjectId,
    key: &[u8],
) -> Option<&'a Object> {
    let mut cur = page_id;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_TREE_DEPTH {
        if !seen.insert(cur) {
            break;
        }
        let dict = doc.get_object(cur).ok()?.as_dict().ok()?;
        if let Ok(val) = dict.get(key) {
            return Some(val);
        }
        match dict.get(b"Parent").ok()? {
            Object::Reference(parent) => cur = *parent,
            _ => break,
        }
    }
    None
}

/// Read `(width_pt, height_pt)` from the page's `/CropBox` or `/MediaBox` (with `/Parent` inheritance).
pub(crate) fn page_dimensions_pt(doc: &Document, page_id: ObjectId) -> (f64, f64) {
    for key in [b"CropBox".as_slice(), b"MediaBox".as_slice()] {
        if let Some(obj) = inherited_page_attr(doc, page_id, key) {
            let arr = match obj {
                Object::Array(a) => Some(a),
                Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_array().ok()),
                _ => None,
            };
            if let Some(a) = arr {
                if a.len() >= 4 {
                    if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
                        obj_as_f64(&a[0]),
                        obj_as_f64(&a[1]),
                        obj_as_f64(&a[2]),
                        obj_as_f64(&a[3]),
                    ) {
                        let w = (x1 - x0).abs();
                        let h = (y1 - y0).abs();
                        if w.is_finite() && h.is_finite() && w > 1.0 && h > 1.0 {
                            return (w, h);
                        }
                    }
                }
            }
        }
    }
    (612.0, 792.0)
}

#[derive(Default)]
struct PageXObjects {
    images: BTreeSet<Vec<u8>>,
    forms: Vec<(Vec<u8>, ObjectId)>,
}

fn classify_xobjects(doc: &Document, resources: &Dictionary) -> PageXObjects {
    let mut out = PageXObjects::default();
    let Some(xobj_raw) = resources.get(b"XObject").ok() else {
        return out;
    };
    let Some(xobjs) = resolve_dict(doc, xobj_raw) else {
        return out;
    };
    for (name, val) in xobjs.iter() {
        let Some(sub_dict) = resolve_dict(doc, val) else {
            continue;
        };
        match sub_dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) {
            Some(b"Image") => {
                out.images.insert(name.clone());
            }
            Some(b"Form") => {
                if let Object::Reference(id) = val {
                    out.forms.push((name.clone(), *id));
                }
            }
            _ => {}
        }
    }
    out
}

fn scan_content_ops(
    doc: &Document,
    raw_content: &[u8],
    resources: Option<&Dictionary>,
    initial_ctm: Affine2d,
    page_area: f64,
    depth: usize,
    signals: &mut PageStreamSignals,
) {
    let Ok(decoded) = Content::decode(raw_content) else {
        return;
    };
    let PageXObjects { images, forms } = resources
        .map(|r| classify_xobjects(doc, r))
        .unwrap_or_default();

    let mut ctm = initial_ctm;
    let mut stack: Vec<Affine2d> = Vec::new();
    let mut text_render_mode: i64 = 0;

    for op in decoded.operations.iter().take(MAX_CONTENT_OPS) {
        match op.operator.as_str() {
            "q" => {
                if stack.len() < 64 {
                    stack.push(ctm);
                }
            }
            "Q" => {
                if let Some(prev) = stack.pop() {
                    ctm = prev;
                }
            }
            "cm" if op.operands.len() >= 6 => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                    obj_as_f64(&op.operands[0]),
                    obj_as_f64(&op.operands[1]),
                    obj_as_f64(&op.operands[2]),
                    obj_as_f64(&op.operands[3]),
                    obj_as_f64(&op.operands[4]),
                    obj_as_f64(&op.operands[5]),
                ) {
                    ctm = ctm.concat(Affine2d { a, b, c, d, e, f });
                }
            }
            "Do" if !op.operands.is_empty() => {
                if let Ok(name) = op.operands[0].as_name() {
                    if images.contains(name) {
                        let cov = (ctm.unit_square_area() / page_area).clamp(0.0, 1.0);
                        if cov > signals.max_image_coverage {
                            signals.max_image_coverage = cov;
                        }
                        signals.total_image_coverage = (signals.total_image_coverage + cov).min(1.0);
                    } else if depth < 2 {
                        if let Some((_, form_id)) = forms.iter().find(|(n, _)| n.as_slice() == name) {
                            if let Ok(Object::Stream(stream)) = doc.get_object(*form_id) {
                                let form_res = stream
                                    .dict
                                    .get(b"Resources")
                                    .ok()
                                    .and_then(|o| resolve_dict(doc, o))
                                    .or(resources);
                                if let Ok(bytes) = stream.decompressed_content() {
                                    scan_content_ops(doc, &bytes, form_res, ctm, page_area, depth + 1, signals);
                                }
                            }
                        }
                    }
                }
            }
            "BI" => {
                signals.has_inline_image = true;
            }
            "Tr" if !op.operands.is_empty() => {
                if let Ok(mode) = op.operands[0].as_i64() {
                    text_render_mode = mode;
                }
            }
            "Tj" | "TJ" | "'" | "\"" => {
                signals.total_text_ops += 1;
                if text_render_mode == 3 {
                    signals.invisible_text_ops += 1;
                }
            }
            _ => {}
        }
    }
}

/// Inspect a PDF page's resources and content stream for scan signals (image coverage, `3 Tr`).
pub(crate) fn inspect_page_signals(doc: &Document, page_id: ObjectId) -> PageStreamSignals {
    let (width_pt, height_pt) = page_dimensions_pt(doc, page_id);
    let page_area = (width_pt * height_pt).max(1.0);
    let mut signals = PageStreamSignals { width_pt, height_pt, ..PageStreamSignals::default() };
    let resources = inherited_page_attr(doc, page_id, b"Resources").and_then(|o| resolve_dict(doc, o));
    if let Ok(content_bytes) = doc.get_page_content(page_id) {
        scan_content_ops(doc, &content_bytes, resources, Affine2d::identity(), page_area, 0, &mut signals);
    }
    signals
}

/// Classify a page as `text_layer`, `scan_text_layer`, or `none` (00-overview.md section 2.12).
///
/// Conservative rule: when unsure, a page counts as a scan rather than a plain text layer.
pub(crate) fn classify_page_basis(
    text: &str,
    signals: &PageStreamSignals,
) -> (PageBasis, PageQualityMetrics, Option<String>) {
    let char_count = text.chars().filter(|c| !c.is_whitespace()).count();
    let alpha_count = text.chars().filter(|c| c.is_alphabetic()).count();
    let word_count = text
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count();
    let image_coverage_pct = (signals.total_image_coverage * 100.0).round().clamp(0.0, 100.0) as u8;
    let has_invisible_text = signals.has_invisible_text();
    let metrics = PageQualityMetrics {
        char_count,
        alpha_count,
        word_count,
        image_coverage_pct,
        has_invisible_text,
        width_pt: signals.width_pt.round().max(1.0) as u32,
        height_pt: signals.height_pt.round().max(1.0) as u32,
    };

    if char_count == 0 {
        let reason = if signals.total_image_coverage >= 0.35 || signals.has_inline_image {
            "image-only page with no text layer"
        } else {
            "page has no extractable text"
        };
        return (PageBasis::None, metrics, Some(reason.into()));
    }

    let std_pages = ((signals.width_pt * signals.height_pt) / STANDARD_PAGE_AREA_PT2).max(0.1);
    let chars_per_std_page = char_count as f64 / std_pages;

    if signals.total_image_coverage >= 0.70 {
        if alpha_count < 15 {
            return (
                PageBasis::None,
                metrics,
                Some("full-page image with negligible text".into()),
            );
        }
        return (
            PageBasis::ScanTextLayer,
            metrics,
            Some("full-page image with text layer".into()),
        );
    }

    if has_invisible_text {
        if alpha_count < 15 {
            return (
                PageBasis::None,
                metrics,
                Some("invisible text layer with negligible text".into()),
            );
        }
        return (
            PageBasis::ScanTextLayer,
            metrics,
            Some("invisible OCR text rendering mode (3 Tr)".into()),
        );
    }

    if (signals.total_image_coverage >= 0.35 || signals.has_inline_image) && chars_per_std_page < 100.0 {
        if alpha_count < 15 {
            return (
                PageBasis::None,
                metrics,
                Some("large image with negligible text".into()),
            );
        }
        return (
            PageBasis::ScanTextLayer,
            metrics,
            Some("large image with sparse text (treated conservatively as scan)".into()),
        );
    }

    if char_count < 15 && chars_per_std_page < 20.0 && alpha_count < 8 {
        return (
            PageBasis::None,
            metrics,
            Some("too little text for page size".into()),
        );
    }

    (PageBasis::TextLayer, metrics, None)
}
