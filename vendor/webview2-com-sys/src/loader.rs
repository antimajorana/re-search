//! Source-built WebView2 discovery for Wry. Runtime layout and the internal creation
//! signature follow webview/webview (MIT); see README.md and LICENSE-LOADER.
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{
    FreeLibrary, E_INVALIDARG, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, S_OK,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows::Win32::System::Com::CoTaskMemAlloc;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
};
use windows_core::{s, w, HRESULT, PCWSTR, PWSTR};

type CreateEnvironment =
    unsafe extern "system" fn(bool, i32, PCWSTR, *mut c_void, *mut c_void) -> HRESULT;
type Result<T> = std::result::Result<T, HRESULT>;
const NOT_FOUND: HRESULT = HRESULT::from_win32(2);

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}

fn registered_root(hive: HKEY) -> Option<PathBuf> {
    let key =
        w!("SOFTWARE\\Microsoft\\EdgeUpdate\\ClientState\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}");
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6432KEY;
    let mut bytes = 0;
    unsafe {
        RegGetValueW(
            hive,
            key,
            w!("EBWebView"),
            flags,
            None,
            None,
            Some(&mut bytes),
        )
        .ok()
        .ok()?;
        if bytes < 4 || bytes > 65_536 || bytes % 2 != 0 {
            return None;
        }
        let mut data = vec![0u16; bytes as usize / 2];
        RegGetValueW(
            hive,
            key,
            w!("EBWebView"),
            flags,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()
        .ok()?;
        let end = data.iter().position(|&v| v == 0)?;
        Some(PathBuf::from(OsString::from_wide(&data[..end])))
    }
}

fn file_version(path: &Path) -> Option<[u32; 4]> {
    let name = wide(path.as_os_str());
    unsafe {
        let len = GetFileVersionInfoSizeW(PCWSTR(name.as_ptr()), None);
        if len == 0 || len > 1024 * 1024 {
            return None;
        }
        let mut data = vec![0u32; (len as usize).div_ceil(4)];
        GetFileVersionInfoW(PCWSTR(name.as_ptr()), None, len, data.as_mut_ptr().cast()).ok()?;
        let mut info = std::ptr::null_mut();
        let mut size = 0;
        if !VerQueryValueW(data.as_ptr().cast(), w!("\\"), &mut info, &mut size).as_bool()
            || info.is_null()
            || (size as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        let v = info.cast::<VS_FIXEDFILEINFO>().read_unaligned();
        Some([
            v.dwFileVersionMS >> 16,
            v.dwFileVersionMS & 0xffff,
            v.dwFileVersionLS >> 16,
            v.dwFileVersionLS & 0xffff,
        ])
    }
}

struct Runtime {
    dll: PathBuf,
    version: [u32; 4],
    fixed: bool,
}

fn runtime_at(root: PathBuf, fixed: bool) -> Option<Runtime> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => return None,
    };
    if !root.is_absolute() {
        return None;
    }
    let dll = root
        .join("EBWebView")
        .join(arch)
        .join("EmbeddedBrowserWebView.dll");
    let version = file_version(&dll)?;
    (version[2] >= 1150).then_some(Runtime {
        dll,
        version,
        fixed,
    })
}

unsafe fn runtime(browser: PCWSTR) -> Result<Runtime> {
    let configured = std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER")
        .filter(|p| !p.is_empty())
        .or_else(|| {
            (!browser.is_null())
                .then(|| OsString::from_wide(browser.as_wide()))
                .filter(|p| !p.is_empty())
        });
    if let Some(folder) = configured {
        return runtime_at(PathBuf::from(folder), true).ok_or(NOT_FOUND);
    }
    [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .filter_map(registered_root)
        .find_map(|root| runtime_at(root, false))
        .ok_or(NOT_FOUND)
}

fn create_fn(path: &Path) -> Result<CreateEnvironment> {
    // Keep the module alive for COM callbacks that outlive the initial function call.
    static FUNCTIONS: OnceLock<Mutex<HashMap<PathBuf, CreateEnvironment>>> = OnceLock::new();
    let mut loaded = FUNCTIONS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| HRESULT::from_win32(31))?;
    if let Some(create) = loaded.get(path) {
        return Ok(*create);
    }
    let name = wide(path.as_os_str());
    unsafe {
        let module = LoadLibraryExW(
            PCWSTR(name.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
        .map_err(|e| e.code())?;
        let Some(address) =
            GetProcAddress(module, s!("CreateWebViewEnvironmentWithOptionsInternal"))
        else {
            let _ = FreeLibrary(module);
            return Err(HRESULT::from_win32(127));
        };
        let create =
            std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateEnvironment>(address);
        loaded.insert(path.to_owned(), create);
        Ok(create)
    }
}

#[inline(never)]
pub(crate) unsafe fn CreateCoreWebView2EnvironmentWithOptions(
    browser: PCWSTR,
    data: PCWSTR,
    options: *mut c_void,
    callback: *mut c_void,
) -> HRESULT {
    if callback.is_null() {
        return E_POINTER;
    }
    let rt = match runtime(browser) {
        Ok(rt) => rt,
        Err(hr) => return hr,
    };
    let create = match create_fn(&rt.dll) {
        Ok(create) => create,
        Err(hr) => return hr,
    };
    let override_dir = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER")
        .filter(|v| !v.is_empty())
        .map(|s| wide(&s));
    let data = override_dir.as_ref().map_or(data, |s| PCWSTR(s.as_ptr()));
    create(true, i32::from(rt.fixed), data, options, callback)
}

pub(crate) unsafe fn CreateCoreWebView2Environment(callback: *mut c_void) -> HRESULT {
    CreateCoreWebView2EnvironmentWithOptions(
        PCWSTR::null(),
        PCWSTR::null(),
        std::ptr::null_mut(),
        callback,
    )
}

pub(crate) unsafe fn GetAvailableCoreWebView2BrowserVersionString(
    browser: PCWSTR,
    out: *mut PWSTR,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = PWSTR::null();
    let rt = match runtime(browser) {
        Ok(rt) => rt,
        Err(hr) => return hr,
    };
    let [a, b, c, d] = rt.version;
    let value = wide(OsStr::new(&format!("{a}.{b}.{c}.{d}")));
    let dest = CoTaskMemAlloc(value.len() * 2).cast::<u16>();
    if dest.is_null() {
        return E_OUTOFMEMORY;
    }
    std::ptr::copy_nonoverlapping(value.as_ptr(), dest, value.len());
    *out = PWSTR(dest);
    S_OK
}

pub(crate) unsafe fn GetAvailableCoreWebView2BrowserVersionStringWithOptions(
    browser: PCWSTR,
    options: *mut c_void,
    out: *mut PWSTR,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = PWSTR::null();
    if !options.is_null() {
        return E_NOTIMPL;
    }
    GetAvailableCoreWebView2BrowserVersionString(browser, out)
}

fn parse_version(s: &str) -> Option<[u32; 4]> {
    let mut result = [0; 4];
    let numeric = s.split_ascii_whitespace().next()?;
    for (i, part) in numeric.split('.').enumerate() {
        *result.get_mut(i)? = part.parse().ok()?;
    }
    Some(result)
}

pub(crate) unsafe fn CompareBrowserVersions(
    first: PCWSTR,
    second: PCWSTR,
    out: *mut i32,
) -> HRESULT {
    if first.is_null() || second.is_null() || out.is_null() {
        return E_POINTER;
    }
    let left = parse_version(&String::from_utf16_lossy(first.as_wide()));
    let right = parse_version(&String::from_utf16_lossy(second.as_wide()));
    match (left, right) {
        (Some(left), Some(right)) => {
            *out = left.cmp(&right) as i32;
            S_OK
        }
        _ => E_INVALIDARG,
    }
}
