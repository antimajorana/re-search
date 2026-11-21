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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroize;

pub const DIR: &str = "chatgpt";
const REGISTRATION_FILE: &str = "registration.dpapi";
const CREDENTIALS_FILE: &str = "credentials.dpapi";
/// Optional DPAPI entropy: binds the blobs to this use.
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

pub fn supported() -> bool {
    cfg!(windows)
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

fn read_decrypted(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let blob = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("the saved ChatGPT sign-in could not be read".into()),
    };
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
        let written = (|| {
            let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            f.write_all(&blob)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, d.join(name))
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

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    /// Copies DPAPI's output and frees it (zeroing the plaintext copy first).
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
        LocalFree(out.pbData as _);
        v
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
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
        let input = blob(sealed);
        let entropy = blob(ENTROPY);
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip_and_files_hold_no_plaintext() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Credentials {
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
        };
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
