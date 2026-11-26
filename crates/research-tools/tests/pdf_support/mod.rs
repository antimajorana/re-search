//! Minimal pure-Rust PDF generator for self-authored F14a test fixtures.
//!
//! Produces valid PDF 1.4 documents strictly from text written in these tests (no third-party
//! PDFs): single-column, two-column (sequential and interleaved stream order), ToUnicode ligatures
//! and soft hyphens, blank/image-only scans, publisher scans with invisible `3 Tr` or normal text,
//! tiled image scans, `/Info` metadata dictionaries, and encrypted/oversized inputs.

#![allow(dead_code)]

use std::io::Write;

#[derive(Debug, Clone)]
pub struct TextOp {
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    pub text: String,
    pub invisible: bool,
}

#[derive(Debug, Clone)]
pub struct ImageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct PageSpec {
    pub width: f64,
    pub height: f64,
    pub texts: Vec<TextOp>,
    pub images: Vec<ImageRect>,
    pub raw_extra_ops: Option<String>,
}

impl Default for PageSpec {
    fn default() -> Self {
        Self {
            width: 612.0,
            height: 792.0,
            texts: Vec::new(),
            images: Vec::new(),
            raw_extra_ops: None,
        }
    }
}

impl PageSpec {
    pub fn single_column(lines: &[&str]) -> Self {
        let mut p = Self::default();
        let mut y = 720.0;
        for line in lines {
            p.texts.push(TextOp {
                x: 72.0,
                y,
                font_size: 12.0,
                text: (*line).to_string(),
                invisible: false,
            });
            y -= 16.0;
        }
        p
    }

    pub fn blank() -> Self {
        Self::default()
    }

    pub fn full_page_image_only() -> Self {
        let mut p = Self::default();
        p.images.push(ImageRect {
            x: 0.0,
            y: 0.0,
            width: 612.0,
            height: 792.0,
        });
        p
    }

    pub fn scan_with_text(lines: &[&str], invisible: bool) -> Self {
        let mut p = Self::full_page_image_only();
        let mut y = 720.0;
        for line in lines {
            p.texts.push(TextOp {
                x: 72.0,
                y,
                font_size: 12.0,
                text: (*line).to_string(),
                invisible,
            });
            y -= 16.0;
        }
        p
    }

    pub fn tiled_scan_with_text(lines: &[&str]) -> Self {
        let mut p = Self::default();
        for strip in 0..4 {
            p.images.push(ImageRect {
                x: 10.0,
                y: strip as f64 * 190.0,
                width: 590.0,
                height: 185.0,
            });
        }
        let mut y = 720.0;
        for line in lines {
            p.texts.push(TextOp {
                x: 72.0,
                y,
                font_size: 12.0,
                text: (*line).to_string(),
                invisible: false,
            });
            y -= 16.0;
        }
        p
    }

    /// Two-column page with optional full-width header lines, left column lines, and right column
    /// lines. When `interleaved` is true, stream ops alternate left/right by row (the layout that
    /// mixes columns in naive stream-order extractors).
    pub fn two_column(
        header: &[&str],
        left_col: &[&str],
        right_col: &[&str],
        interleaved: bool,
    ) -> Self {
        let mut p = Self::default();
        let mut y = 740.0;
        for h in header {
            p.texts.push(TextOp {
                x: 72.0,
                y,
                font_size: 14.0,
                text: (*h).to_string(),
                invisible: false,
            });
            y -= 20.0;
        }
        let col_top = y - 10.0;
        let left_x = 54.0;
        let right_x = 324.0;
        let rows = left_col.len().max(right_col.len());
        if interleaved {
            for r in 0..rows {
                let row_y = col_top - (r as f64) * 15.0;
                if let Some(l) = left_col.get(r) {
                    p.texts.push(TextOp {
                        x: left_x,
                        y: row_y,
                        font_size: 10.0,
                        text: (*l).to_string(),
                        invisible: false,
                    });
                }
                if let Some(rt) = right_col.get(r) {
                    p.texts.push(TextOp {
                        x: right_x,
                        y: row_y,
                        font_size: 10.0,
                        text: (*rt).to_string(),
                        invisible: false,
                    });
                }
            }
        } else {
            for (r, l) in left_col.iter().enumerate() {
                p.texts.push(TextOp {
                    x: left_x,
                    y: col_top - (r as f64) * 15.0,
                    font_size: 10.0,
                    text: (*l).to_string(),
                    invisible: false,
                });
            }
            for (r, rt) in right_col.iter().enumerate() {
                p.texts.push(TextOp {
                    x: right_x,
                    y: col_top - (r as f64) * 15.0,
                    font_size: 10.0,
                    text: (*rt).to_string(),
                    invisible: false,
                });
            }
        }
        p
    }
}

#[derive(Debug, Clone, Default)]
pub struct PdfBuildOptions {
    pub info_title_utf16: Option<String>,
    pub info_author: Option<String>,
    pub info_subject: Option<String>,
    pub info_creation_date: Option<String>,
    pub encrypted_stub: bool,
}

/// Encode text for our Type1 font with custom `/ToUnicode` mapping:
/// `0x01 -> U+FB01 (fi)`, `0x02 -> U+FB02 (fl)`, `0x03 -> U+FB03 (ffi)`,
/// `0x04 -> U+00AD (soft hyphen)`, ASCII `0x20..=0x7E` as-is.
fn encode_hex_text(s: &str) -> String {
    let mut hex = String::with_capacity(s.len() * 2 + 2);
    hex.push('<');
    for c in s.chars() {
        let b: u8 = match c {
            '\u{FB01}' => 0x01,
            '\u{FB02}' => 0x02,
            '\u{FB03}' => 0x03,
            '\u{00AD}' => 0x04,
            c if (0x20..=0x7E).contains(&(c as u32)) => c as u8,
            _ => b'?',
        };
        hex.push_str(&format!("{b:02X}"));
    }
    hex.push('>');
    hex
}

fn to_unicode_cmap() -> &'static str {
    concat!(
        "/CIDInit /ProcSet findresource begin\n",
        "12 dict begin\n",
        "begincmap\n",
        "/CIDSystemInfo << /Registry (Synthetic) /Ordering (UCS) /Supplement 0 >> def\n",
        "/CMapName /Synthetic-UTF16 def\n",
        "/CMapType 2 def\n",
        "1 begincodespacerange\n",
        "<00> <FF>\n",
        "endcodespacerange\n",
        "4 beginbfchar\n",
        "<01> <FB01>\n",
        "<02> <FB02>\n",
        "<03> <FB03>\n",
        "<04> <00AD>\n",
        "endbfchar\n",
        "1 beginbfrange\n",
        "<20> <7E> <0020>\n",
        "endbfrange\n",
        "endcmap\n",
        "CMapName currentdict /CMap defineresource pop\n",
        "end\n",
        "end\n",
    )
}

fn encode_utf16be_hex(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16() {
        out.push_str(&format!("{u:04X}"));
    }
    out.push('>');
    out
}

fn escape_pdf_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            _ => out.push(c),
        }
    }
    out
}

pub fn build_pdf(pages: &[PageSpec], opts: &PdfBuildOptions) -> Vec<u8> {
    // Fixed object IDs:
    // 1: Catalog, 2: Pages, 3: ToUnicode CMap, 4: Font /F1, 5: 1x1 RGB Image XObject /Im1
    // Optional: Info dict, Encrypt dict; then 2 objects per page (Page dict + Content stream).
    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();

    let cmap = to_unicode_cmap();
    let img_data = b"\xC0\xC0\xC0"; // 1x1 RGB grey pixel

    let mut next_id = 6usize;
    let info_id = if opts.info_title_utf16.is_some()
        || opts.info_author.is_some()
        || opts.info_subject.is_some()
        || opts.info_creation_date.is_some()
    {
        let id = next_id;
        next_id += 1;
        Some(id)
    } else {
        None
    };

    let encrypt_id = if opts.encrypted_stub {
        let id = next_id;
        next_id += 1;
        Some(id)
    } else {
        None
    };

    let first_page_obj = next_id;
    let kids: String = (0..pages.len())
        .map(|i| format!("{} 0 R", first_page_obj + i * 2))
        .collect::<Vec<_>>()
        .join(" ");

    objs.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    objs.push((
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).into_bytes(),
    ));
    objs.push((
        3,
        format!("<< /Length {} >>\nstream\n{}\nendstream", cmap.len(), cmap).into_bytes(),
    ));
    objs.push((
        4,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 3 0 R >>".to_vec(),
    ));
    objs.push((
        5,
        format!(
            "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
            img_data.len()
        )
        .into_bytes()
        .into_iter()
        .chain(img_data.iter().copied())
        .chain(b"\nendstream".iter().copied())
        .collect(),
    ));

    if let Some(id) = info_id {
        let mut dict = String::from("<<");
        if let Some(t) = &opts.info_title_utf16 {
            dict.push_str(&format!(" /Title {}", encode_utf16be_hex(t)));
        }
        if let Some(a) = &opts.info_author {
            dict.push_str(&format!(" /Author ({})", escape_pdf_literal(a)));
        }
        if let Some(s) = &opts.info_subject {
            dict.push_str(&format!(" /Subject ({})", escape_pdf_literal(s)));
        }
        if let Some(d) = &opts.info_creation_date {
            dict.push_str(&format!(" /CreationDate ({})", escape_pdf_literal(d)));
        }
        dict.push_str(" >>");
        objs.push((id, dict.into_bytes()));
    }

    if let Some(id) = encrypt_id {
        // Non-empty owner/user password stub so lopdf marks the document encrypted and rejects "".
        let o_hex = "01".repeat(32);
        let u_hex = "02".repeat(32);
        let enc = format!(
            "<< /Filter /Standard /V 1 /R 2 /Length 40 /O <{o_hex}> /U <{u_hex}> /P -4 >>"
        );
        objs.push((id, enc.into_bytes()));
    }

    for (i, page) in pages.iter().enumerate() {
        let page_id = first_page_obj + i * 2;
        let content_id = page_id + 1;

        let mut stream = String::new();
        for img in &page.images {
            stream.push_str(&format!(
                "q {} 0 0 {} {} {} cm /Im1 Do Q\n",
                img.width, img.height, img.x, img.y
            ));
        }
        for t in &page.texts {
            let tr = if t.invisible { 3 } else { 0 };
            let hex = encode_hex_text(&t.text);
            stream.push_str(&format!(
                "BT /F1 {} Tf {} Tr 1 0 0 1 {} {} Tm {} Tj ET\n",
                t.font_size, tr, t.x, t.y, hex
            ));
        }
        if let Some(extra) = &page.raw_extra_ops {
            stream.push_str(extra);
            stream.push('\n');
        }

        let xobj_res = if page.images.is_empty() {
            ""
        } else {
            " /XObject << /Im1 5 0 R >>"
        };
        let page_dict = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 4 0 R >>{} >> /Contents {} 0 R >>",
            page.width, page.height, xobj_res, content_id
        );
        objs.push((page_id, page_dict.into_bytes()));
        objs.push((
            content_id,
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                stream.len(),
                stream
            )
            .into_bytes(),
        ));
    }

    let mut out = b"%PDF-1.4\n".to_vec();
    let max_id = objs.iter().map(|(id, _)| *id).max().unwrap_or(0);
    let mut offsets = vec![0usize; max_id + 1];
    for (id, body) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    let total_xref = max_id + 1;
    writeln!(out, "xref\n0 {total_xref}\n0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    let mut trailer = format!("<< /Size {total_xref} /Root 1 0 R");
    if let Some(id) = info_id {
        trailer.push_str(&format!(" /Info {id} 0 R"));
    }
    if let Some(id) = encrypt_id {
        trailer.push_str(&format!(" /Encrypt {id} 0 R"));
        trailer.push_str(" /ID [<AABBCCDD> <AABBCCDD>]");
    }
    trailer.push_str(" >>");
    writeln!(out, "trailer\n{trailer}\nstartxref\n{xref_pos}\n%%EOF").unwrap();
    out
}
