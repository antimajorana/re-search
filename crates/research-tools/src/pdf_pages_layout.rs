//! Layout-aware `pdf_extract::OutputDev` with two-column vertical gutter splitting (package F14a).

use pdf_extract::{MediaBox, OutputDev, OutputError, Transform};

const MAX_GLYPHS_PER_PAGE: usize = 250_000;

#[derive(Debug, Clone)]
struct Glyph {
    x: f64,
    y: f64,
    x_end: f64,
    font_size: f64,
    text: String,
}

#[derive(Debug, Clone)]
struct Piece {
    x_start: f64,
    x_end: f64,
    y: f64,
    text: String,
}

/// Layout-aware `OutputDev` that collects positioned glyphs and splits two-column pages along a
/// detected vertical gutter before assembling top-to-bottom reading lines.
pub(crate) struct LayoutOutputDev {
    flip_ctm: Transform,
    page_width: f64,
    glyphs: Vec<Glyph>,
}

impl LayoutOutputDev {
    pub fn new() -> Self {
        Self {
            flip_ctm: Transform::identity(),
            page_width: 612.0,
            glyphs: Vec::new(),
        }
    }

    pub fn into_text(self) -> String {
        if self.glyphs.is_empty() {
            return String::new();
        }
        let median_fs = median_f64(self.glyphs.iter().map(|g| g.font_size).collect()).clamp(4.0, 72.0);
        let pieces = build_pieces(self.glyphs, median_fs);
        if pieces.is_empty() {
            return String::new();
        }
        if let Some(split_x) = detect_two_column_gutter(&pieces, self.page_width, median_fs) {
            emit_two_column_text(&pieces, split_x, median_fs)
        } else {
            emit_single_column_text(&pieces, median_fs)
        }
    }
}

impl OutputDev for LayoutOutputDev {
    fn begin_page(
        &mut self,
        _page_num: u32,
        media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        let w = (media_box.urx - media_box.llx).abs();
        self.page_width = if w.is_finite() && w > 1.0 { w } else { 612.0 };
        self.flip_ctm = Transform::row_major(1.0, 0.0, 0.0, -1.0, -media_box.llx, media_box.ury);
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        ch: &str,
    ) -> Result<(), OutputError> {
        if self.glyphs.len() >= MAX_GLYPHS_PER_PAGE || ch.is_empty() {
            return Ok(());
        }
        let clean: String = ch
            .chars()
            .filter(|&c| c != '\0')
            .map(|c| if c == '\x0c' { ' ' } else { c })
            .collect();
        if clean.is_empty() {
            return Ok(());
        }
        let pos = trm.post_transform(&self.flip_ctm);
        let (x, y) = (pos.m31, pos.m32);
        if !x.is_finite() || !y.is_finite() {
            return Ok(());
        }
        let sx = (trm.m11 * font_size).hypot(trm.m12 * font_size);
        let sy = (trm.m21 * font_size).hypot(trm.m22 * font_size);
        let fs_raw = (sx * sy).sqrt();
        let fs = if fs_raw.is_finite() && fs_raw >= 1.0 {
            fs_raw
        } else if font_size.is_finite() && font_size.abs() >= 1.0 {
            font_size.abs()
        } else {
            10.0
        };
        let adv = if width.is_finite() { (width * fs).abs() } else { fs * 0.5 };
        self.glyphs.push(Glyph { x, y, x_end: x + adv, font_size: fs, text: clean });
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

fn median_f64(mut vals: Vec<f64>) -> f64 {
    vals.retain(|v| v.is_finite());
    if vals.is_empty() {
        return 10.0;
    }
    vals.sort_by(|a, b| a.total_cmp(b));
    vals[vals.len() / 2]
}

fn build_pieces(mut glyphs: Vec<Glyph>, median_fs: f64) -> Vec<Piece> {
    glyphs.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));
    let y_tol = (median_fs * 0.45).max(2.0);
    let mut bands: Vec<Vec<Glyph>> = Vec::new();
    for g in glyphs {
        if let Some(last_band) = bands.last_mut() {
            let band_y = last_band[0].y;
            if (g.y - band_y).abs() <= y_tol {
                last_band.push(g);
                continue;
            }
        }
        bands.push(vec![g]);
    }

    let mut pieces = Vec::new();
    for mut band in bands {
        band.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut cur: Option<Piece> = None;
        for g in band {
            let gap_split = (g.font_size.max(median_fs) * 1.4).max(12.0);
            let space_thresh = g.font_size.max(median_fs) * 0.14;
            match cur.as_mut() {
                Some(p) if g.x - p.x_end <= gap_split => {
                    if g.x > p.x_end + space_thresh && !p.text.ends_with(' ') && !g.text.starts_with(' ') {
                        p.text.push(' ');
                    }
                    p.text.push_str(&g.text);
                    if g.x_end > p.x_end {
                        p.x_end = g.x_end;
                    }
                }
                _ => {
                    if let Some(done) = cur.take() {
                        if !done.text.trim().is_empty() {
                            pieces.push(done);
                        }
                    }
                    cur = Some(Piece {
                        x_start: g.x,
                        x_end: g.x_end,
                        y: g.y,
                        text: g.text,
                    });
                }
            }
        }
        if let Some(done) = cur {
            if !done.text.trim().is_empty() {
                pieces.push(done);
            }
        }
    }
    pieces
}

fn detect_two_column_gutter(pieces: &[Piece], page_width: f64, median_fs: f64) -> Option<f64> {
    let narrow: Vec<&Piece> = pieces
        .iter()
        .filter(|p| {
            (p.x_end - p.x_start) < page_width * 0.55
                && p.text.chars().filter(|c| c.is_alphanumeric()).count() >= 2
        })
        .collect();
    if narrow.len() < 6 {
        return None;
    }

    let x_min = page_width * 0.35;
    let x_max = page_width * 0.65;
    let step = 2.0_f64;
    let margin = 1.5_f64;

    let mut best_run: Option<(f64, f64, usize)> = None;
    let mut cur_start: Option<f64> = None;
    let mut cur_max_cross = 0usize;

    let better = |cand: (f64, f64, usize), best: (f64, f64, usize)| -> bool {
        let cw = cand.1 - cand.0;
        let bw = best.1 - best.0;
        (usize::MAX - cand.2, cw.to_bits()) > (usize::MAX - best.2, bw.to_bits())
    };

    let mut x = x_min;
    while x <= x_max {
        let mut crossing = 0usize;
        let mut left = 0usize;
        let mut right = 0usize;
        for p in &narrow {
            if p.x_start < x - margin && p.x_end > x + margin {
                crossing += 1;
            } else if p.x_end <= x + margin {
                left += 1;
            } else if p.x_start >= x - margin {
                right += 1;
            }
        }
        let max_cross = ((left + right) / 4).clamp(1, 3);
        if crossing <= max_cross && left >= 3 && right >= 3 {
            if cur_start.is_none() {
                cur_start = Some(x);
                cur_max_cross = crossing;
            } else {
                cur_max_cross = cur_max_cross.max(crossing);
            }
        } else if let Some(start) = cur_start.take() {
            let cand = (start, x - step, cur_max_cross);
            if best_run.is_none_or(|b| better(cand, b)) {
                best_run = Some(cand);
            }
        }
        x += step;
    }
    if let Some(start) = cur_start {
        let cand = (start, x_max, cur_max_cross);
        if best_run.is_none_or(|b| better(cand, b)) {
            best_run = Some(cand);
        }
    }

    let (gx0, gx1, _) = best_run?;
    let min_gutter = (median_fs * 1.0).max(6.0);
    if gx1 - gx0 < min_gutter {
        return None;
    }
    let split_x = (gx0 + gx1) * 0.5;

    let (mut l_min, mut l_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut r_min, mut r_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &narrow {
        if p.x_end <= split_x + margin {
            l_min = l_min.min(p.y);
            l_max = l_max.max(p.y);
        } else if p.x_start >= split_x - margin {
            r_min = r_min.min(p.y);
            r_max = r_max.max(p.y);
        }
    }
    let overlap = l_max.min(r_max) - l_min.max(r_min);
    if overlap >= median_fs * 0.5 { Some(split_x) } else { None }
}

fn emit_lines(mut pieces: Vec<&Piece>, median_fs: f64, out: &mut Vec<String>) {
    if pieces.is_empty() {
        return;
    }
    pieces.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x_start.total_cmp(&b.x_start)));
    let y_tol = (median_fs * 0.45).max(2.0);
    let mut line_y = pieces[0].y;
    let mut line_buf = pieces[0].text.trim().to_string();

    for p in pieces.into_iter().skip(1) {
        let t = p.text.trim();
        if t.is_empty() {
            continue;
        }
        if (p.y - line_y).abs() <= y_tol {
            if !line_buf.is_empty() {
                line_buf.push(' ');
            }
            line_buf.push_str(t);
        } else {
            if !line_buf.is_empty() {
                out.push(line_buf);
            }
            line_y = p.y;
            line_buf = t.to_string();
        }
    }
    if !line_buf.is_empty() {
        out.push(line_buf);
    }
}

fn emit_single_column_text(pieces: &[Piece], median_fs: f64) -> String {
    let mut out = Vec::new();
    emit_lines(pieces.iter().collect(), median_fs, &mut out);
    out.join("\n")
}

fn emit_two_column_text(pieces: &[Piece], split_x: f64, median_fs: f64) -> String {
    let margin = 1.5_f64;
    let mut ordered: Vec<&Piece> = pieces.iter().collect();
    ordered.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x_start.total_cmp(&b.x_start)));

    let mut out = Vec::new();
    let mut left_band: Vec<&Piece> = Vec::new();
    let mut right_band: Vec<&Piece> = Vec::new();

    let flush_band = |left: &mut Vec<&Piece>, right: &mut Vec<&Piece>, out: &mut Vec<String>| {
        if !left.is_empty() {
            emit_lines(std::mem::take(left), median_fs, out);
        }
        if !right.is_empty() {
            emit_lines(std::mem::take(right), median_fs, out);
        }
    };

    for p in ordered {
        let crosses = p.x_start < split_x - margin && p.x_end > split_x + margin;
        if crosses {
            flush_band(&mut left_band, &mut right_band, &mut out);
            emit_lines(vec![p], median_fs, &mut out);
        } else if (p.x_start + p.x_end) * 0.5 < split_x {
            left_band.push(p);
        } else {
            right_band.push(p);
        }
    }
    flush_band(&mut left_band, &mut right_band, &mut out);
    out.join("\n")
}
