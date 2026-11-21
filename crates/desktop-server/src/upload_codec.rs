//! Base64 and multipart/form-data upload decoding helpers.

pub(crate) type UploadedFile = (String, Vec<u8>);

/// Decodes standard or URL-safe base64 strings (whitespace ignored, optional `=` padding).
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    let mut ended = false;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            ended = true;
            continue;
        }
        if ended {
            return Err("unexpected data after base64 padding".to_string());
        }
        let val: u32 = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("invalid base64 byte: 0x{b:02x}")),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

pub(crate) fn parse_multipart_files(
    content_type: &str,
    body: &[u8],
) -> Result<Vec<UploadedFile>, (u16, String)> {
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
        .map(|b| b.trim().trim_matches('"').to_string())
        .filter(|b| !b.is_empty())
        .ok_or_else(|| (400, "multipart/form-data missing boundary".to_string()))?;
    let sep = format!("--{boundary}").into_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = find_bytes(&body[pos..], &sep) {
        let start = pos + rel + sep.len();
        if start + 2 <= body.len() && &body[start..start + 2] == b"--" {
            break;
        }
        let part_start = if start + 2 <= body.len() && &body[start..start + 2] == b"\r\n" {
            start + 2
        } else if start < body.len() && body[start] == b'\n' {
            start + 1
        } else {
            start
        };
        let next = match find_bytes(&body[part_start..], &sep) {
            Some(n) => part_start + n,
            None => break,
        };
        let mut part = &body[part_start..next];
        if part.ends_with(b"\r\n") {
            part = &part[..part.len() - 2];
        } else if part.ends_with(b"\n") {
            part = &part[..part.len() - 1];
        }
        if let Some(hdr_end) = find_bytes(part, b"\r\n\r\n") {
            let hdrs = String::from_utf8_lossy(&part[..hdr_end]);
            let data = &part[hdr_end + 4..];
            for line in hdrs.lines() {
                if line.to_ascii_lowercase().starts_with("content-disposition:") {
                    for seg in line.split(';') {
                        if let Some(val) = seg.trim().strip_prefix("filename=") {
                            out.push((val.trim().trim_matches('"').to_string(), data.to_vec()));
                            break;
                        }
                    }
                }
            }
        }
        pos = next;
    }
    Ok(out)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
