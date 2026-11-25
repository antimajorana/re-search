use super::*;
use crate::Ledger;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempTestDir(PathBuf);

impl TempTestDir {
    fn new(label: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "re-search-ftcache-{label}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn put_and_get_by_key_and_version_roundtrip() {
    let tmp = TempTestDir::new("roundtrip");
    let cache = FulltextCache::open(&tmp.0).unwrap();
    let payload_v1 = b"\\documentclass{article}\\begin{document}Version one.\\end{document}";
    let payload_v2 = b"\\documentclass{article}\\begin{document}Version two.\\end{document}";

    let e1 = cache
        .put_bytes("arxiv:2103.01234", Some("v1"), "tex", "arxiv_eprint", payload_v1)
        .unwrap();
    let e2 = cache
        .put_bytes("arxiv:2103.01234", Some("v2"), "tex", "arxiv_eprint", payload_v2)
        .unwrap();

    assert_eq!(e1.raw_blob, sha256_hex(payload_v1));
    assert_eq!(e2.raw_blob, sha256_hex(payload_v2));
    assert!(tmp.0.join(LIBRARY_DIR_NAME).join(LIBRARY_DB_NAME).is_file());

    let (got1, bytes1) = cache.get_bytes("arxiv:2103.01234", Some("v1")).unwrap().unwrap();
    assert_eq!(got1.version.as_deref(), Some("v1"));
    assert_eq!(bytes1, payload_v1);

    let (got2, bytes2) = cache.get_bytes("arxiv:2103.01234", Some("v2")).unwrap().unwrap();
    assert_eq!(got2.version.as_deref(), Some("v2"));
    assert_eq!(bytes2, payload_v2);

    // Unknown version is a miss.
    assert!(cache.get_bytes("arxiv:2103.01234", Some("v3")).unwrap().is_none());

    // Unversioned lookup hits an existing cached version (latest by timestamp/version).
    let (any, any_bytes) = cache.get_bytes("arxiv:2103.01234", None).unwrap().unwrap();
    assert_eq!(any.version.as_deref(), Some("v2"));
    assert_eq!(any_bytes, payload_v2);
}

#[test]
fn prefers_tex_over_pdf_on_unversioned_lookup() {
    let tmp = TempTestDir::new("prefer-tex");
    let cache = FulltextCache::open(&tmp.0).unwrap();
    let pdf = b"%PDF-1.4\nfake pdf";
    let tex = b"\\section{Real}";

    cache
        .put_bytes("arxiv:2103.01234", Some("v1"), "tex", "arxiv_eprint", tex)
        .unwrap();
    cache
        .put_bytes("arxiv:2103.01234", Some("v2"), "pdf", "arxiv_pdf", pdf)
        .unwrap();

    // Exact version v2 returns the PDF, while unversioned prefers TeX over PDF.
    let (v2, _) = cache.get_bytes("arxiv:2103.01234", Some("v2")).unwrap().unwrap();
    assert_eq!(v2.format, "pdf");

    let (any, bytes) = cache.get_bytes("arxiv:2103.01234", None).unwrap().unwrap();
    assert_eq!(any.format, "tex");
    assert_eq!(bytes, tex);
}

#[test]
fn reuses_ledger_blob_without_creating_unreferenced_ledger_blobs() {
    let tmp = TempTestDir::new("ledger-reuse");
    let ledger = Ledger::open(&tmp.0).unwrap();
    let cache = FulltextCache::open(&tmp.0).unwrap();

    // Storing via put_bytes writes to <data_dir>/library/blobs/, leaving ledger blobs untouched.
    let raw = b"\\section{Cached only}";
    cache
        .put_bytes("arxiv:2103.09999", Some("v1"), "tex", "arxiv_eprint", raw)
        .unwrap();
    let report = ledger.verify_all().unwrap();
    assert_eq!(report.blobs_verified, 0);
    assert_eq!(report.unreferenced_blobs, 0);

    // Indexing an existing ledger blob reads straight from <data_dir>/blobs/.
    let blob_bytes = b"\\section{From ledger}";
    let hash = ledger.put_blob(blob_bytes).unwrap();
    cache
        .index_existing_blob(
            "arxiv:2103.08888",
            Some("v1"),
            &hash,
            "tex",
            blob_bytes.len() as u64,
            "arxiv_eprint",
        )
        .unwrap();
    let (entry, read_back) = cache.get_bytes("arxiv:2103.08888", Some("v1")).unwrap().unwrap();
    assert_eq!(entry.raw_blob, hash);
    assert_eq!(read_back, blob_bytes);
}

#[test]
fn corrupted_cache_blob_is_treated_as_cache_miss() {
    let tmp = TempTestDir::new("corrupt");
    let cache = FulltextCache::open(&tmp.0).unwrap();
    let raw = b"\\section{Integrity check}";
    let entry = cache
        .put_bytes("arxiv:2103.05555", Some("v1"), "tex", "arxiv_eprint", raw)
        .unwrap();

    // Tamper with the rebuildable cache blob on disk.
    let blob_path = tmp
        .0
        .join(LIBRARY_DIR_NAME)
        .join("blobs")
        .join(&entry.raw_blob[..2])
        .join(&entry.raw_blob);
    std::fs::write(&blob_path, b"corrupted bytes").unwrap();

    assert!(cache.get_bytes("arxiv:2103.05555", Some("v1")).unwrap().is_none());
}

#[test]
fn remove_deletes_requested_entries() {
    let tmp = TempTestDir::new("remove");
    let cache = FulltextCache::open(&tmp.0).unwrap();
    cache
        .put_bytes("arxiv:2103.01111", Some("v1"), "tex", "arxiv_eprint", b"v1")
        .unwrap();
    cache
        .put_bytes("arxiv:2103.01111", Some("v2"), "tex", "arxiv_eprint", b"v2")
        .unwrap();

    assert_eq!(cache.remove("arxiv:2103.01111", Some("v1")).unwrap(), 1);
    assert!(cache.get_entry("arxiv:2103.01111", Some("v1")).unwrap().is_none());
    assert!(cache.get_entry("arxiv:2103.01111", Some("v2")).unwrap().is_some());

    assert_eq!(cache.remove("arxiv:2103.01111", None).unwrap(), 1);
    assert!(cache.get_entry("arxiv:2103.01111", None).unwrap().is_none());
}
