//! Concurrent-safe fingerprinted Python runtime staging and path guardrails.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

const STAGE_LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const STALE_LOCK_AGE: Duration = Duration::from_secs(15 * 60);
const ALLOWED_PACKAGES: &[&str] = &["sympy", "mpmath", "numpy", "numpy.libs"];
const ALLOWED_DIST_PREFIXES: &[&str] = &["sympy-", "mpmath-", "numpy-"];
static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct StagedRuntime {
    pub root: PathBuf,
    pub python_exe: PathBuf,
    pub site_dir: PathBuf,
    pub worker_script: PathBuf,
}

/// Strip Windows verbatim path prefixes (`\\?\UNC\` -> `\\`, `\\?\` -> ``).
pub fn strip_verbatim(p: &Path) -> PathBuf {
    let wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    let unc = [b'\\' as u16, b'\\'.into(), b'?'.into(), b'\\'.into(), b'U'.into(), b'N'.into(), b'C'.into(), b'\\'.into()];
    if wide.len() >= unc.len() && wide[..4] == unc[..4] && matches_ascii_ci(&wide[4..8], &unc[4..8]) {
        let mut out = vec![b'\\' as u16, b'\\' as u16];
        out.extend_from_slice(&wide[8..]);
        return PathBuf::from(std::ffi::OsString::from_wide(&out));
    }
    let verbatim = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    if wide.starts_with(&verbatim) {
        return PathBuf::from(std::ffi::OsString::from_wide(&wide[4..]));
    }
    p.to_path_buf()
}

fn matches_ascii_ci(a: &[u16], upper: &[u16]) -> bool {
    a.len() == upper.len()
        && a.iter()
            .zip(upper.iter())
            .all(|(&x, &y)| x <= 0x7f && u16::from((x as u8).to_ascii_uppercase()) == y)
}

fn normalized_components(path: &Path) -> Vec<String> {
    strip_verbatim(path)
        .components()
        .map(|c| match c {
            Component::Prefix(p) => p.as_os_str().to_string_lossy().to_lowercase(),
            Component::RootDir => "\\".to_string(),
            Component::CurDir => ".".to_string(),
            Component::ParentDir => "..".to_string(),
            Component::Normal(s) => s.to_string_lossy().to_lowercase(),
        })
        .collect()
}

/// Return `true` iff canonicalized `target` is strictly inside canonicalized `root`
/// (`target != root && target.starts_with(root)` by path components).
pub fn is_strictly_inside(target: &Path, root: &Path) -> io::Result<bool> {
    let t = normalized_components(&fs::canonicalize(target)?);
    let r = normalized_components(&fs::canonicalize(root)?);
    Ok(t.len() > r.len() && t.starts_with(&r))
}

pub(crate) fn is_same_or_inside(target: &Path, root: &Path) -> io::Result<bool> {
    let t = normalized_components(&fs::canonicalize(target)?);
    let r = normalized_components(&fs::canonicalize(root)?);
    Ok(t.len() >= r.len() && t.starts_with(&r))
}

/// Lexically check that `target` has no `..` traversal and sits strictly under `root`.
pub(crate) fn lexical_strictly_inside(target: &Path, root: &Path) -> io::Result<bool> {
    if target.components().any(|c| matches!(c, Component::ParentDir)) {
        return Ok(false);
    }
    let t = normalized_components(&std::path::absolute(target)?);
    let r = normalized_components(&std::path::absolute(root)?);
    Ok(t.len() > r.len() && t.starts_with(&r))
}

pub(crate) fn lexical_same_or_inside(target: &Path, root: &Path) -> io::Result<bool> {
    if target.components().any(|c| matches!(c, Component::ParentDir)) {
        return Ok(false);
    }
    let t = normalized_components(&std::path::absolute(target)?);
    let r = normalized_components(&std::path::absolute(root)?);
    Ok(t.len() >= r.len() && t.starts_with(&r))
}

/// Grant an `icacls.exe` ACE on `target`, refusing any path not strictly inside `data_dir`.
pub fn grant_icacls_inside(data_dir: &Path, target: &Path, grant_spec: &str) -> io::Result<()> {
    if !is_strictly_inside(target, data_dir)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing ACL change outside data_dir: {}", target.display()),
        ));
    }
    let clean = strip_verbatim(&fs::canonicalize(target)?);
    let sys_root = std::env::var_os("SystemRoot")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let icacls_exe = sys_root.join("System32").join("icacls.exe");
    let out = Command::new(&icacls_exe)
        .arg(&clean)
        .arg("/grant")
        .arg(grant_spec)
        .arg("/Q")
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "icacls failed ({}) on {}: {}",
            out.status,
            clean.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

struct LockGuard(PathBuf);
impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct TempDirGuard(Option<PathBuf>);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

fn acquire_stage_lock(lock_path: &Path) -> io::Result<LockGuard> {
    let start = Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(lock_path) {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                return Ok(LockGuard(lock_path.to_path_buf()));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(lock_path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > STALE_LOCK_AGE);
                if stale {
                    let _ = fs::remove_file(lock_path);
                    continue;
                }
                if start.elapsed() >= STAGE_LOCK_TIMEOUT {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("timed out waiting for staging lock {}", lock_path.display()),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}

struct PythonProbe {
    base: PathBuf,
    ver: String,
    site_dirs: Vec<PathBuf>,
}

fn probe_source_python(source_python: &Path) -> io::Result<PythonProbe> {
    let out = Command::new(source_python)
        .args([
            "-I",
            "-c",
            "import json, os, sys; print(json.dumps({'base': os.path.realpath(sys.base_prefix), 'ver': sys.version, 'paths': sys.path}))",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "python probe failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad python probe JSON: {e}")))?;
    let base = PathBuf::from(v["base"].as_str().ok_or_else(|| io::Error::other("missing base"))?);
    let ver = v["ver"].as_str().ok_or_else(|| io::Error::other("missing ver"))?.to_string();

    let mut seen = BTreeSet::new();
    let mut site_dirs = Vec::new();
    let candidates = v["paths"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).map(PathBuf::from);
    for cand in candidates {
        let is_site = cand.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("site-packages"));
        if is_site && cand.is_dir() {
            if let Ok(canon) = fs::canonicalize(&cand) {
                if seen.insert(normalized_components(&canon)) {
                    site_dirs.push(strip_verbatim(&canon));
                }
            }
        }
    }
    if site_dirs.is_empty() {
        if let Some(fallback) = source_python.parent().and_then(|p| p.parent()).map(|p| p.join("Lib").join("site-packages")) {
            if fallback.is_dir() {
                site_dirs.push(strip_verbatim(&fs::canonicalize(fallback)?));
            }
        }
    }
    Ok(PythonProbe { base, ver, site_dirs })
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn compute_fingerprint(probe: &PythonProbe, source_python: &Path, worker_bytes: &[u8]) -> io::Result<String> {
    let mut h = Sha256::new();
    let update_field = |h: &mut Sha256, s: &str| {
        h.update((s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    update_field(&mut h, "stage-v1");
    update_field(&mut h, &probe.base.to_string_lossy());
    update_field(&mut h, &probe.ver);
    let canon_py = strip_verbatim(&fs::canonicalize(source_python)?);
    update_field(&mut h, &canon_py.to_string_lossy());
    update_field(&mut h, &hex_digest(worker_bytes));
    let mut dist_infos = BTreeSet::new();
    for sp in &probe.site_dirs {
        if let Ok(rd) = fs::read_dir(sp) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if matches_package_entry(&name) {
                    dist_infos.insert(name.to_lowercase());
                }
            }
        }
    }
    for name in dist_infos {
        update_field(&mut h, &name);
    }
    Ok(hex_digest(&h.finalize())[..16].to_string())
}

fn matches_package_entry(name: &str) -> bool {
    let low = name.to_ascii_lowercase();
    ALLOWED_PACKAGES.contains(&low.as_str())
        || (low.ends_with(".dist-info") && ALLOWED_DIST_PREFIXES.iter().any(|p| low.starts_with(p)))
}

fn copy_file_with_mtime(src: &Path, dst: &Path) -> io::Result<()> {
    fs::copy(src, dst)?;
    if let Ok(mtime) = fs::metadata(src).and_then(|m| m.modified()) {
        if let Ok(f) = File::options().write(true).open(dst) {
            let _ = f.set_modified(mtime);
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path, skip_lib_site: bool, depth: usize) -> io::Result<()> {
    if depth > 64 {
        return Err(io::Error::other(format!("directory recursion limit exceeded at {}", src.display())));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_low = name.to_string_lossy().to_ascii_lowercase();
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let meta = fs::metadata(&src_path)?;
        if meta.is_dir() {
            if skip_lib_site && depth == 0 && matches!(name_low.as_str(), "envs" | "pkgs" | "conda-meta") {
                continue;
            }
            if skip_lib_site && depth == 1 && name_low == "site-packages" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path, skip_lib_site, depth + 1)?;
        } else if meta.is_file() {
            copy_file_with_mtime(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn staged_paths(target: &Path) -> Option<StagedRuntime> {
    let ready = target.join("ready.json");
    let py = target.join("py").join("python.exe");
    let script = target.join("worker.py");
    if ready.is_file() && py.is_file() && script.is_file() {
        let root = strip_verbatim(&fs::canonicalize(target).ok()?);
        return Some(StagedRuntime {
            python_exe: root.join("py").join("python.exe"),
            site_dir: root.join("site"),
            worker_script: root.join("worker.py"),
            root,
        });
    }
    None
}

/// Stage `source_python`'s base runtime, selected math packages, and `worker_script`
/// under `<data_dir>/runtime/<fingerprint>/` with `*S-1-15-2-1:(OI)(CI)RX`.
pub fn ensure_staged_runtime(data_dir: &Path, source_python: &Path, worker_script: &Path) -> io::Result<StagedRuntime> {
    if !source_python.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("python not found: {}", source_python.display())));
    }
    if !worker_script.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("worker script not found: {}", worker_script.display())));
    }
    let probe = probe_source_python(source_python)?;
    let worker_bytes = fs::read(worker_script)?;
    let fp = compute_fingerprint(&probe, source_python, &worker_bytes)?;
    let runtime_root = data_dir.join("runtime");
    let target = runtime_root.join(&fp);
    if let Some(staged) = staged_paths(&target) {
        return Ok(staged);
    }

    fs::create_dir_all(&runtime_root)?;
    let _lock = acquire_stage_lock(&runtime_root.join(".stage.lock"))?;
    if let Some(staged) = staged_paths(&target) {
        return Ok(staged);
    }

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_dir = runtime_root.join(format!(".tmp-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_dir);
    let mut tmp_guard = TempDirGuard(Some(tmp_dir.clone()));

    copy_dir_recursive(&probe.base, &tmp_dir.join("py"), true, 0)?;
    if !tmp_dir.join("py").join("python.exe").is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "staged base_prefix does not contain python.exe"));
    }
    let site_dst = tmp_dir.join("site");
    fs::create_dir_all(&site_dst)?;
    for sp in &probe.site_dirs {
        for entry in fs::read_dir(sp)? {
            let entry = entry?;
            let name = entry.file_name();
            if !matches_package_entry(&name.to_string_lossy()) {
                continue;
            }
            let dst_entry = site_dst.join(&name);
            if dst_entry.exists() {
                continue;
            }
            let src_entry = entry.path();
            if fs::metadata(&src_entry)?.is_dir() {
                copy_dir_recursive(&src_entry, &dst_entry, false, 0)?;
            } else {
                copy_file_with_mtime(&src_entry, &dst_entry)?;
            }
        }
    }
    copy_file_with_mtime(worker_script, &tmp_dir.join("worker.py"))?;

    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::rename(&tmp_dir, &target)?;
    tmp_guard.0 = None;

    grant_icacls_inside(data_dir, &target, "*S-1-15-2-1:(OI)(CI)RX")?;

    let ready_tmp = target.join(".ready.tmp");
    let meta = serde_json::json!({ "fingerprint": fp, "base": probe.base, "ver": probe.ver });
    fs::write(&ready_tmp, serde_json::to_vec(&meta).map_err(|e| io::Error::other(e.to_string()))?)?;
    fs::rename(&ready_tmp, target.join("ready.json"))?;

    staged_paths(&target).ok_or_else(|| io::Error::other("staged runtime missing expected files after staging"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_verbatim_handles_drive_and_unc_prefixes() {
        assert_eq!(strip_verbatim(Path::new(r"\\?\C:\work\a\b")), PathBuf::from(r"C:\work\a\b"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\UNC\server\share\dir")), PathBuf::from(r"\\server\share\dir"));
        assert_eq!(strip_verbatim(Path::new(r"C:\plain\path")), PathBuf::from(r"C:\plain\path"));
    }

    #[test]
    fn strictly_inside_rejects_self_and_sibling_prefixes() {
        let base = std::env::temp_dir().join(format!("stage-guard-test-{}", std::process::id()));
        let root = base.join("data");
        let sibling = base.join("data2");
        let child = root.join("sessions").join("s1");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&sibling).unwrap();

        assert!(!is_strictly_inside(&root, &root).unwrap());
        assert!(!is_strictly_inside(&sibling, &root).unwrap());
        assert!(is_strictly_inside(&child, &root).unwrap());
        assert!(grant_icacls_inside(&root, &sibling, "*S-1-15-2-1:(OI)(CI)RX").is_err());

        let _ = fs::remove_dir_all(&base);
    }
}
