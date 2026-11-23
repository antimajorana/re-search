//! Integration tests for F4 (`doc_ingest`, `pdf.rs`, `archive.rs`, `tex_bundle.rs`,
//! and `"source.ingested"` `"raw_blob"` provenance).

use flate2::write::GzEncoder;
use flate2::Compression;
use ledger::Ledger;
use research_tools::archive::{extract_gzip_or_tar_gz, extract_tar_bytes, extract_zip_bytes, ArchiveError};
use research_tools::pdf::{extract_pdf_bytes, PdfError, PdfExtractStatus};
use research_tools::{
    assemble_tex_bundle, register_research_tools, research_policy, ResearchEnv, DOC_INGEST, FS_READ,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::sync::Arc;
use tempfile::TempDir;
use tools::approval::{ApprovalReply, ApprovalRequest, Approver};
use tools::router::{CallOutcome, Router};
use tools::Registry;
use zip::write::SimpleFileOptions;

struct DenyAskApprover;
impl Approver for DenyAskApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: false, digest: req.digest.clone(), by: "test".into() }
    }
}

/// Builds a genuine minimal valid PDF with `num_pages` pages and optional text stream on page 1.
fn build_minimal_pdf(num_pages: usize, first_page_text: Option<&str>) -> Vec<u8> {
    // Object IDs: 1 = Catalog, 2 = Pages, 3 = Font (Helvetica), 4 = Content stream (optional),
    // Page objects start at 5 .. 5 + num_pages.
    let has_text = first_page_text.is_some();
    let first_page_id = if has_text { 5usize } else { 4usize };
    let kids: String = (0..num_pages)
        .map(|i| format!("{} 0 R", first_page_id + i))
        .collect::<Vec<_>>()
        .join(" ");

    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();
    objs.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    objs.push((
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {num_pages} >>").into_bytes(),
    ));
    if let Some(txt) = first_page_text {
        objs.push((3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()));
        let stream_body = format!("BT /F1 12 Tf 72 720 Td ({txt}) Tj ET");
        let stream_obj = format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            stream_body.len(),
            stream_body
        );
        objs.push((4, stream_obj.into_bytes()));
    } else {
        objs.push((3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()));
    }

    for i in 0..num_pages {
        let pid = first_page_id + i;
        let page_dict = if i == 0 && has_text {
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".to_string()
        } else {
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string()
        };
        objs.push((pid, page_dict.into_bytes()));
    }

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objs.len() + 1];
    for (id, body) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    let total_xref = objs.len() + 1;
    writeln!(out, "xref\n0 {total_xref}\n0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(
        out,
        "trailer\n<< /Size {total_xref} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF"
    )
    .unwrap();
    out
}

fn build_zip(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cur);
        for (name, data, mode) in entries {
            let mut opts = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            if let Some(m) = mode {
                opts = opts.unix_permissions(*m);
            }
            zw.start_file(*name, opts).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }
    cur.into_inner()
}

fn gzip_bytes(raw: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(raw).unwrap();
    enc.finish().unwrap()
}

#[test]
fn f4_pdf_valid_scanned_caps_and_malformed_safety() {
    // 1. Valid PDF text extraction.
    let valid_pdf = build_minimal_pdf(1, Some("Eigenvalue bound lambda_1 > 0"));
    let res = extract_pdf_bytes(&valid_pdf).expect("extract valid PDF");
    assert_eq!(res.status, PdfExtractStatus::Ok);
    assert_eq!(res.page_count, 1);
    assert!(res.text.contains("Eigenvalue bound lambda_1 > 0"));

    // 2. Scanned/blank PDF -> EmptyTextLayer with empty text + warning.
    let blank_pdf = build_minimal_pdf(2, None);
    let empty_res = extract_pdf_bytes(&blank_pdf).expect("extract blank PDF");
    assert_eq!(empty_res.status, PdfExtractStatus::EmptyTextLayer);
    assert_eq!(empty_res.page_count, 2);
    assert!(empty_res.text.is_empty());
    assert!(empty_res.warning.is_some());

    // 3. > 400 pages rejected before stream decoding.
    let huge_pages_pdf = build_minimal_pdf(401, None);
    assert_eq!(
        extract_pdf_bytes(&huge_pages_pdf),
        Err(PdfError::TooManyPages { pages: 401, max: 400 })
    );

    // 4. > 16 MiB rejected immediately.
    let oversized = vec![b'%'; 16 * 1024 * 1024 + 1];
    assert!(matches!(extract_pdf_bytes(&oversized), Err(PdfError::TooLarge { .. })));

    // 5. Malformed PDF without /Root or broken xref does not panic (catch_unwind / clean Err).
    let broken = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< >>\n%%EOF";
    assert!(extract_pdf_bytes(broken).is_err());
}

#[test]
fn f4_archive_anti_bomb_traversal_symlinks_zip64_and_latin1_gz() {
    // 1. ZIP path traversal including `foo/../bar` (which ZipFile::enclosed_name allows!) & `../evil`.
    for bad_name in ["../evil.tex", "foo/../../evil.tex", "foo/../bar.tex", "/abs.tex", "a\\b.tex"] {
        let z = build_zip(&[(bad_name, b"hi", None)]);
        assert!(
            matches!(extract_zip_bytes(&z), Err(ArchiveError::UnsafeEntry(_))),
            "expected `{bad_name}` to be rejected"
        );
    }

    // 2. ZIP symlink entry (via ZipWriter::add_symlink) & special FIFO mode rejected.
    let mut sym_cur = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut sym_cur);
        zw.add_symlink("link.tex", "../outside.tex", SimpleFileOptions::default()).unwrap();
        zw.finish().unwrap();
    }
    let z_sym = sym_cur.into_inner();
    assert!(matches!(extract_zip_bytes(&z_sym), Err(ArchiveError::UnsafeEntry(_))));

    // 3. ZIP64 EOCD locator marker rejected.
    let mut z64 = build_zip(&[("a.tex", b"ok", None)]);
    z64.extend_from_slice(b"PK\x06\x07\x00\x00\x00\x00");
    assert_eq!(extract_zip_bytes(&z64), Err(ArchiveError::Zip64Unsupported));

    // 4. Encrypted ZIP bit (general purpose bit 0 in central + local headers) rejected.
    let mut z_enc = build_zip(&[("secret.tex", b"hello", None)]);
    for i in 0..z_enc.len().saturating_sub(10) {
        if &z_enc[i..i + 4] == b"PK\x03\x04" {
            z_enc[i + 6] |= 0x01;
        } else if &z_enc[i..i + 4] == b"PK\x01\x02" {
            z_enc[i + 8] |= 0x01;
        }
    }
    assert_eq!(extract_zip_bytes(&z_enc), Err(ArchiveError::EncryptedArchive));

    // 5. > 16 MiB single entry deflate bomb & > 256 files rejected via Read::take.
    let zeros_17mb = vec![b'A'; 16 * 1024 * 1024 + 64];
    let z_bomb = build_zip(&[("bomb.tex", &zeros_17mb, None)]);
    assert!(matches!(extract_zip_bytes(&z_bomb), Err(ArchiveError::LimitExceeded(_))));

    // > 32 MiB cumulative across 3 x 11 MiB entries.
    let chunk_11mb = vec![b'B'; 11 * 1024 * 1024];
    let z_cum = build_zip(&[
        ("p1.tex", &chunk_11mb, None),
        ("p2.tex", &chunk_11mb, None),
        ("p3.tex", &chunk_11mb, None),
    ]);
    assert!(matches!(extract_zip_bytes(&z_cum), Err(ArchiveError::LimitExceeded(_))));

    let many_names: Vec<String> = (0..257).map(|i| format!("f{i}.tex")).collect();
    let many_refs: Vec<(&str, &[u8], Option<u32>)> =
        many_names.iter().map(|n| (n.as_str(), &b"x"[..], None)).collect();
    let z_many = build_zip(&many_refs);
    assert!(matches!(extract_zip_bytes(&z_many), Err(ArchiveError::LimitExceeded(_))));

    // 6. TAR symlink, hardlink, FIFO, and `foo/../bar` raw header rejection.
    for kind in [tar::EntryType::Symlink, tar::EntryType::Link, tar::EntryType::Fifo] {
        let mut tb = tar::Builder::new(Vec::new());
        let mut hdr = tar::Header::new_gnu();
        hdr.set_entry_type(kind);
        hdr.set_size(0);
        hdr.set_mode(0o644);
        if matches!(kind, tar::EntryType::Symlink | tar::EntryType::Link) {
            hdr.set_link_name("target.tex").unwrap();
        }
        hdr.set_cksum();
        tb.append_data(&mut hdr, "special_entry", std::io::empty()).unwrap();
        let tar_bytes = tb.into_inner().unwrap();
        assert!(matches!(extract_tar_bytes(&tar_bytes), Err(ArchiveError::UnsafeEntry(_))));
    }

    // Raw TAR header containing `foo/../bar.tex` (bypassing tar::Builder's client-side check).
    let mut raw_tar = vec![0u8; 1536];
    let bad_tar_path = b"foo/../bar.tex";
    raw_tar[..bad_tar_path.len()].copy_from_slice(bad_tar_path);
    raw_tar[100..107].copy_from_slice(b"0000644");
    raw_tar[124..135].copy_from_slice(b"00000000002");
    raw_tar[156] = b'0';
    raw_tar[257..262].copy_from_slice(b"ustar");
    raw_tar[148..156].fill(b' ');
    let cksum: u32 = raw_tar[..512].iter().map(|&b| b as u32).sum();
    let ck_str = format!("{cksum:06o}\0 ");
    raw_tar[148..156].copy_from_slice(ck_str.as_bytes());
    raw_tar[512..514].copy_from_slice(b"hi");
    assert!(matches!(extract_tar_bytes(&raw_tar), Err(ArchiveError::UnsafeEntry(_))));

    // 7. Outer GZIP > 32 MiB anti-bomb & single-file ISO-8859-1 `.gz` preprint decoding.
    let gz_bomb = gzip_bytes(&vec![b'Z'; 32 * 1024 * 1024 + 128]);
    assert!(matches!(
        extract_gzip_or_tar_gz(&gz_bomb, "bomb.gz", false),
        Err(ArchiveError::LimitExceeded(_))
    ));

    // "Schrödinger" in ISO-8859-1 (`0xF6` = 'ö').
    let latin1_tex = b"\\section{Schr\xf6dinger operator}\n";
    let gz_latin1 = gzip_bytes(latin1_tex);
    match extract_gzip_or_tar_gz(&gz_latin1, "9901001.gz", false).unwrap() {
        research_tools::archive::ArchivePayload::SingleGzipText { suggested_name, text } => {
            assert_eq!(suggested_name, "9901001.tex");
            assert!(text.contains("Schrödinger operator"));
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[test]
fn f4_tex_bundle_scoring_inlining_cycles_and_provenance_zero_unreferenced_blobs() {
    // 1. Direct unit check of TeX bundle scoring, `+{.tex}` resolution, `%` vs `\%`, `\includegraphics`, cycle & depth=5.
    let mut tex_map = BTreeMap::new();
    tex_map.insert("ignored_comment.tex".into(), "FORBIDDEN_COMMENT_BODY\n".into());
    tex_map.insert("sub/ghost.tex".into(), "FORBIDDEN_GHOST_BODY\n".into());
    tex_map.insert(
        "sub/helper.tex".into(),
        "\\section{Helper}\n100\\% exact % \\input{ghost}\n\\includegraphics{plot.png}\n\\input{cycle_a}\n".into(),
    );
    tex_map.insert("sub/cycle_a.tex".into(), "\\input{cycle_b}\n".into());
    tex_map.insert("sub/cycle_b.tex".into(), "\\input{cycle_a}\n".into());
    tex_map.insert(
        "main.tex".into(),
        "\\documentclass{article}\n\\begin{document}\n% \\input{ignored_comment}\n\\include{sub/helper}\n\\end{document}\n".into(),
    );
    let bundle = assemble_tex_bundle(&tex_map).expect("assemble bundle");
    assert_eq!(bundle.primary_path, "main.tex");
    assert!(bundle.primary_score >= 2250);
    assert!(bundle.assembled_tex.contains("\\section{Helper}"));
    assert!(bundle.assembled_tex.contains("100\\% exact"));
    assert!(bundle.assembled_tex.contains("\\includegraphics{plot.png}"));
    assert!(!bundle.assembled_tex.contains("ignored_comment"));
    assert!(!bundle.assembled_tex.contains("ghost"));
    assert!(bundle.assembled_tex.contains("cycle detected"));

    // Depth > 5 cap check.
    let mut deep_map = BTreeMap::new();
    deep_map.insert("main.tex".into(), "\\documentclass{article}\n\\input{d1}\n".into());
    for d in 1..=6 {
        deep_map.insert(format!("d{d}.tex"), format!("level-{d}\n\\input{{d{}}}\n", d + 1));
    }
    deep_map.insert("d7.tex".into(), "level-7\n".into());
    let deep_bundle = assemble_tex_bundle(&deep_map).unwrap();
    assert!(deep_bundle.assembled_tex.contains("level-5"));
    assert!(deep_bundle.assembled_tex.contains("max include depth exceeded"));
    assert!(!deep_bundle.assembled_tex.contains("level-7"));

    // 2. End-to-end Router::try_call across all text formats + PDF + .tar.gz + rejected NUL text.
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();

    for (fname, body) in [
        ("a.tex", "\\section{Intro}\nHello TeX\n"),
        ("b.bib", "@article{k1, title={T}}\n"),
        ("c.bbl", "\\bibitem{k1} Ref\n"),
        ("d.sty", "\\ProvidesPackage{d}\n"),
        ("e.cls", "\\ProvidesClass{e}\n"),
        ("f.md", "# Heading\n"),
        ("g.markdown", "## Sub\n"),
        ("h.txt", "plain text\n"),
        ("i.csv", "a,b\n1,2\n"),
        ("j.tsv", "a\tb\n1\t2\n"),
        ("k.json", "{\"ok\":true}\n"),
        ("l.py", "x = 42\n"),
        ("m.log", "This is pdfTeX\n"),
        ("n.yml", "key: val\n"),
        ("o.yaml", "key2: val2\n"),
    ] {
        fs::write(ws.join(fname), body).unwrap();
    }
    fs::write(ws.join("paper.pdf"), build_minimal_pdf(1, Some("Theorem 1 spectral gap"))).unwrap();
    fs::write(ws.join("corrupt_nul.txt"), b"bad\0text").unwrap();

    // Build a multi-file .tar.gz bundle with main.tex + sec.tex + binary figure.png.
    let mut tb = tar::Builder::new(Vec::new());
    for (p, data) in [
        ("ms.tex", &b"\\documentclass{article}\n\\begin{document}\n\\section{Model}\nHamiltonian $H$\n\\input{sec}\n\\end{document}\n"[..]),
        ("sec.tex", &b"\\section{Proof}\nBy Cauchy-Schwarz.\n"[..]),
        ("fig.png", &b"\x89PNG\r\n\x1a\n\x00\x00binary"[..]),
    ] {
        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(data.len() as u64);
        hdr.set_mode(0o644);
        hdr.set_cksum();
        tb.append_data(&mut hdr, p, data).unwrap();
    }
    let tgz = gzip_bytes(&tb.into_inner().unwrap());
    fs::write(ws.join("bundle.tar.gz"), tgz).unwrap();

    let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap());
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let session = "s-ingestprovenance";
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data).unwrap(),
        session,
        Arc::new(DenyAskApprover),
    );

    for fname in [
        "a.tex", "b.bib", "c.bbl", "d.sty", "e.cls", "f.md", "g.markdown", "h.txt", "i.csv",
        "j.tsv", "k.json", "l.py", "m.log", "n.yml", "o.yaml", "paper.pdf",
    ] {
        let out = router.try_call("agent:test", DOC_INGEST, &json!({"path": fname})).unwrap();
        assert!(matches!(out, CallOutcome::Ok(_)), "failed on {fname}: {out:?}");
    }

    // Rejected NUL text file must NOT leave an unreferenced blob!
    assert!(matches!(
        router.try_call("agent:test", DOC_INGEST, &json!({"path": "corrupt_nul.txt"})).unwrap(),
        CallOutcome::Error(_)
    ));

    // Ingest .tar.gz with section_filter = "Proof" and read materialized __assembled__.tex via fs_read.
    let bundle_out = router
        .try_call(
            "agent:test",
            DOC_INGEST,
            &json!({"path": "bundle.tar.gz", "section_filter": "Proof"}),
        )
        .unwrap();
    let assembled_rel = match bundle_out {
        CallOutcome::Ok(v) => {
            assert_eq!(v["status"], "ok");
            assert_eq!(v["skipped_binary_entries"], json!(["fig.png"]));
            assert!(v["excerpt"].as_str().unwrap().contains("Cauchy-Schwarz"));
            v["assembled_path"].as_str().unwrap().to_string()
        }
        other => panic!("unexpected {other:?}"),
    };
    let read_assembled = router
        .try_call("agent:test", FS_READ, &json!({"root": "sources", "path": assembled_rel}))
        .unwrap();
    assert!(matches!(read_assembled, CallOutcome::Ok(v) if v["content"].as_str().unwrap().contains("Cauchy-Schwarz")));

    // CRITICAL PROVENANCE CHECK: hash chain intact and 0 unreferenced blobs!
    let ledger = Ledger::open(&data).unwrap();
    ledger.verify_session(session).expect("session chain valid");
    let report = ledger.verify_all().expect("verify_all valid");
    assert_eq!(report.unreferenced_blobs, 0);
}

#[test]
fn tex_title_extraction_skips_comments_and_other_title_macros() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();
    fs::write(
        ws.join("titled.tex"),
        "% \\title{Ignored Old Title}\n\\titlerunning{Short}\n\\title{\\textbf{Lieb--Schultz--Mattis} on a Torus}\n\\begin{document}\\end{document}\n",
    )
    .unwrap();
    let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap());
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env);
    let session = "s-tex-title";
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data).unwrap(),
        session,
        Arc::new(DenyAskApprover),
    );
    let out = router.try_call("agent:test", DOC_INGEST, &json!({"path": "titled.tex"})).unwrap();
    match out {
        CallOutcome::Ok(v) => assert_eq!(v["title"], "Lieb--Schultz--Mattis on a Torus"),
        other => panic!("unexpected outcome: {other:?}"),
    }
}
