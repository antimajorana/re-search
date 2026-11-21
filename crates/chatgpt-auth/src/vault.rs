//! Token storage: `<data_dir>/chatgpt/`, each file encrypted with Windows DPAPI
//! (`CryptProtectData`, current user, no UI) and replaced atomically (temp file + rename).
//!
//! - `registration.dpapi`: this host's `ext_agent_host_id` and the issued client ID with
//!   its verified account (kept on sign-out, as the documentation advises).
//! - `credentials.dpapi`: ID, access and refresh tokens, granted scopes and expiry
//!   (deleted on sign-out or when ChatGPT ends the session).
//!
//! On other systems nothing is stored: every write fails with "not supported yet" rather
//! than writing tokens in plain text.

use crate::codec::Secret;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};
use zeroize::Zeroize;

pub const DIR: &str = "chatgpt";
const REGISTRATION_FILE: &str = "registration.dpapi";
const CREDENTIALS_FILE: &str = "credentials.dpapi";
const LOCK_FILE: &str = ".lock";
const MAX_VAULT_FILE: u64 = 1024 * 1024;
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(25);
const LOCK_STALE: Duration = Duration::from_secs(60);
/// Optional DPAPI entropy: binds the blobs to this use.
#[cfg(windows)]
const ENTROPY: &[u8] = b"re:Search chatgpt-auth v1";
pub const NOT_SUPPORTED: &str = "Sign in with ChatGPT stores its tokens with Windows DPAPI; it is not supported on this system yet";

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registration {
    pub ext_agent_host_id: String,
    /// Issued client ID (never `dynamic_agent_client`).
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
}

impl fmt::Debug for Registration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registration").field("client_id", &self.client_id.is_some()).finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub issuer: String,
    pub client_id: String,
    pub subject: String,
    #[serde(default)]
    pub email: Option<String>,
    pub id_token: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub token_type: String,
    /// Unix seconds when the access token expires.
    pub expires_at_s: i64,
    pub scopes: Vec<String>,
    pub saved_at_s: i64,
    #[serde(default)]
    pub earliest_refresh_at: Option<i64>,
}

impl Credentials {
    pub fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == crate::oauth::PLAN_SCOPE)
    }

    pub fn access(&self) -> Secret {
        Secret::new(self.access_token.clone())
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("expires_at_s", &self.expires_at_s)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.id_token.zeroize();
        self.access_token.zeroize();
        if let Some(r) = &mut self.refresh_token {
            r.zeroize();
        }
    }
}

/// Cross-process lock on `<data_dir>/chatgpt/.lock` created atomically with `create_new`.
pub struct VaultLock {
    path: PathBuf,
}

impl VaultLock {
    pub fn acquire(data_dir: &Path) -> Result<Self, String> {
        Self::acquire_with(data_dir, LOCK_WAIT, LOCK_STALE)
    }

    pub(crate) fn acquire_with(data_dir: &Path, wait: Duration, stale: Duration) -> Result<Self, String> {
        let d = dir(data_dir);
        fs::create_dir_all(&d).map_err(|_| "could not create the ChatGPT folder in the data directory".to_string())?;
        let path = d.join(LOCK_FILE);
        let deadline = Instant::now() + wait;
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if is_stale_lock(&path, stale) && fs::remove_file(&path).is_ok() && Instant::now() < deadline {
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err("another ChatGPT sign-in operation is in progress; please try again".into());
                    }
                    std::thread::sleep(LOCK_POLL);
                }
                Err(_) => return Err("could not lock the ChatGPT folder in the data directory".into()),
            }
        }
    }
}

impl Drop for VaultLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn is_stale_lock(path: &Path, stale: Duration) -> bool {
    if let Ok(meta) = fs::metadata(path) {
        if let Ok(modified) = meta.modified() {
            if SystemTime::now().duration_since(modified).is_ok_and(|age| age >= stale) {
                return true;
            }
        }
    }
    let Ok(f) = fs::File::open(path) else {
        return false;
    };
    let mut text = String::new();
    if f.take(32).read_to_string(&mut text).is_err() {
        return false;
    }
    let Ok(pid) = text.trim().parse::<u32>() else {
        return false;
    };
    pid != std::process::id() && !process_alive(pid)
}

#[cfg(target_os = "linux")]
fn process_alive(pid: u32) -> bool {
    if !Path::new("/proc").is_dir() {
        return true;
    }
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    const STILL_ACTIVE_CODE: u32 = 259;
    // SAFETY: OpenProcess returns null on failure or a handle closed before returning.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(h, &mut code) };
    unsafe { CloseHandle(h) };
    ok != 0 && code == STILL_ACTIVE_CODE
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_alive(_pid: u32) -> bool {
    true
}

pub fn supported() -> bool {
    cfg!(windows)
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

fn read_decrypted(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("the saved ChatGPT sign-in could not be read".into()),
    };
    if f.metadata().is_ok_and(|m| m.len() > MAX_VAULT_FILE) {
        return Err("the saved ChatGPT sign-in file is unexpectedly large".into());
    }
    let mut blob = Vec::new();
    if f.take(MAX_VAULT_FILE + 1).read_to_end(&mut blob).is_err() {
        return Err("the saved ChatGPT sign-in could not be read".into());
    }
    if (blob.len() as u64) > MAX_VAULT_FILE {
        return Err("the saved ChatGPT sign-in file is unexpectedly large".into());
    }
    dpapi::unprotect(&blob).map(Some)
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn write_encrypted(data_dir: &Path, name: &str, plain: &mut Vec<u8>) -> Result<(), String> {
    let res = (|| {
        if !supported() {
            return Err(NOT_SUPPORTED.to_string());
        }
        let blob = dpapi::protect(plain)?;
        let d = dir(data_dir);
        fs::create_dir_all(&d).map_err(|_| "could not create the ChatGPT folder in the data directory".to_string())?;
        let tmp = d.join(format!(".{name}.{}.{}.tmp", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
        let dst = d.join(name);
        let written = (|| -> std::io::Result<()> {
            let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            f.write_all(&blob)?;
            f.sync_all()?;
            drop(f);
            let mut attempt = 0u32;
            loop {
                match fs::rename(&tmp, &dst) {
                    Ok(()) => return Ok(()),
                    Err(e)
                        if attempt < 4
                            && (e.kind() == std::io::ErrorKind::PermissionDenied
                                || matches!(e.raw_os_error(), Some(5 | 32))) =>
                    {
                        attempt += 1;
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    Err(e) => return Err(e),
                }
            }
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written.map_err(|_| "could not save the ChatGPT sign-in".to_string())
    })();
    plain.zeroize();
    res
}

pub fn load_registration(data_dir: &Path) -> Result<Option<Registration>, String> {
    match read_decrypted(&dir(data_dir).join(REGISTRATION_FILE))? {
        None => Ok(None),
        Some(mut bytes) => {
            let r = serde_json::from_slice(&bytes).map_err(|_| "the saved ChatGPT registration is damaged".to_string());
            bytes.zeroize();
            r.map(Some)
        }
    }
}

pub fn save_registration(data_dir: &Path, r: &Registration) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(r).map_err(|e| e.to_string())?;
    write_encrypted(data_dir, REGISTRATION_FILE, &mut bytes)
}

pub fn load_credentials(data_dir: &Path) -> Result<Option<Credentials>, String> {
    match read_decrypted(&dir(data_dir).join(CREDENTIALS_FILE))? {
        None => Ok(None),
        Some(mut bytes) => {
            let r = serde_json::from_slice(&bytes).map_err(|_| "the saved ChatGPT sign-in is damaged".to_string());
            bytes.zeroize();
            r.map(Some)
        }
    }
}

pub fn save_credentials(data_dir: &Path, c: &Credentials) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(c).map_err(|e| e.to_string())?;
    write_encrypted(data_dir, CREDENTIALS_FILE, &mut bytes)
}

pub fn delete_credentials(data_dir: &Path) -> Result<(), String> {
    match fs::remove_file(dir(data_dir).join(CREDENTIALS_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not delete the saved ChatGPT tokens".into()),
    }
}

#[cfg(windows)]
mod dpapi {
    use super::ENTROPY;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB, String> {
        let len = u32::try_from(data.len()).map_err(|_| "ChatGPT sign-in data is too large for DPAPI".to_string())?;
        Ok(CRYPT_INTEGER_BLOB { cbData: len, pbData: data.as_ptr() as *mut u8 })
    }

    /// Copies DPAPI's output and frees it (zeroing the plaintext copy first).
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let slice = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let v = slice.to_vec();
        zeroize::Zeroize::zeroize(slice);
        LocalFree(out.pbData as _);
        v
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain)?;
        let entropy = blob(ENTROPY)?;
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: all pointers are valid for the call; `out` is allocated by DPAPI and
        // freed by `take`.
        let ok = unsafe {
            CryptProtectData(&input, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return Err("Windows could not encrypt the ChatGPT sign-in (DPAPI)".into());
        }
        Ok(unsafe { take(out) })
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(sealed)?;
        let entropy = blob(ENTROPY)?;
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: as in `protect`.
        let ok = unsafe {
            CryptUnprotectData(&input, std::ptr::null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return Err("the saved ChatGPT sign-in could not be decrypted for this Windows user".into());
        }
        Ok(unsafe { take(out) })
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, String> {
        Err(super::NOT_SUPPORTED.to_string())
    }

    pub fn unprotect(_sealed: &[u8]) -> Result<Vec<u8>, String> {
        Err(super::NOT_SUPPORTED.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_creds() -> Credentials {
        Credentials {
            issuer: "i".into(),
            client_id: "oaiapp_test".into(),
            subject: "sub-1".into(),
            email: None,
            id_token: "idtok-PLAIN-MARKER".into(),
            access_token: "acc-PLAIN-MARKER".into(),
            refresh_token: Some("ref-PLAIN-MARKER".into()),
            token_type: "Bearer".into(),
            expires_at_s: 10,
            scopes: vec![crate::oauth::PLAN_SCOPE.into()],
            saved_at_s: 1,
            earliest_refresh_at: None,
        }
    }

    #[test]
    fn vault_lock_serializes_and_reclaims_stale_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = VaultLock::acquire(tmp.path()).unwrap();
        assert!(dir(tmp.path()).join(LOCK_FILE).exists());
        // While held by current PID, a short wait must fail rather than steal.
        assert!(VaultLock::acquire_with(tmp.path(), Duration::from_millis(80), LOCK_STALE).is_err());
        drop(lock);
        assert!(!dir(tmp.path()).join(LOCK_FILE).exists());

        // Stale lock by age is reclaimed.
        fs::create_dir_all(dir(tmp.path())).unwrap();
        fs::write(dir(tmp.path()).join(LOCK_FILE), format!("{}\n", std::process::id())).unwrap();
        let reclaimed = VaultLock::acquire_with(tmp.path(), Duration::from_millis(200), Duration::ZERO).unwrap();
        drop(reclaimed);

        // On Linux, a lock owned by a non-existent PID is reclaimed immediately.
        #[cfg(target_os = "linux")]
        {
            fs::write(dir(tmp.path()).join(LOCK_FILE), "999999999\n").unwrap();
            let reclaimed = VaultLock::acquire_with(tmp.path(), Duration::from_millis(200), LOCK_STALE).unwrap();
            drop(reclaimed);
        }
    }

    #[test]
    fn read_decrypted_caps_files_above_one_mib() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(tmp.path());
        fs::create_dir_all(&d).unwrap();
        let path = d.join(CREDENTIALS_FILE);
        fs::write(&path, vec![0u8; (MAX_VAULT_FILE as usize) + 1]).unwrap();
        let err = load_credentials(tmp.path()).unwrap_err();
        assert!(err.contains("large"), "{err}");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_fails_closed_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!supported());
        let reg = Registration { ext_agent_host_id: "urn:uuid:00000000-0000-4000-8000-000000000000".into(), client_id: None, subject: None };
        assert_eq!(save_registration(tmp.path(), &reg).unwrap_err(), NOT_SUPPORTED);
        assert_eq!(save_credentials(tmp.path(), &sample_creds()).unwrap_err(), NOT_SUPPORTED);
        assert!(fs::read_dir(tmp.path()).unwrap().next().is_none(), "must write nothing to disk");
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip_and_files_hold_no_plaintext() {
        let tmp = tempfile::tempdir().unwrap();
        let c = sample_creds();
        save_credentials(tmp.path(), &c).unwrap();
        let raw = fs::read(dir(tmp.path()).join(CREDENTIALS_FILE)).unwrap();
        assert!(!raw.windows(12).any(|w| w == b"PLAIN-MARKER"));
        let back = load_credentials(tmp.path()).unwrap().unwrap();
        assert_eq!(back.access_token, "acc-PLAIN-MARKER");
        assert!(back.plan_enabled());
        assert!(!format!("{back:?}").contains("MARKER"));
        delete_credentials(tmp.path()).unwrap();
        assert!(load_credentials(tmp.path()).unwrap().is_none());
        delete_credentials(tmp.path()).unwrap();
    }
}
