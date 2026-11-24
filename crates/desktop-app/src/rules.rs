//! Pure lock-down rules for the app window: which navigations are allowed, which downloads
//! are allowed, and where an allowed download is saved. Unit-tested without a display.

use std::path::{Path, PathBuf};

/// `true` only for `about:blank` and for URLs on exactly this server's origin.
///
/// `base_url` is `http://127.0.0.1:<port>` (no trailing slash). The character after it must
/// end the authority (`/`, `?`, `#` or end of string), so `http://127.0.0.1:<port>0`,
/// `http://127.0.0.1:<port>.evil.example` and `http://127.0.0.1:<port>@evil.example` fail.
pub fn is_allowed_navigation(url: &str, base_url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    match url.strip_prefix(base_url) {
        Some(rest) => rest.is_empty() || rest.starts_with(['/', '?', '#']),
        None => false,
    }
}

const ALLOWED_EXTERNAL_HOSTS: &[&str] = &[
    "arxiv.org",
    "export.arxiv.org",
    "doi.org",
    "dx.doi.org",
    "inspirehep.net",
    "semanticscholar.org",
    "www.semanticscholar.org",
    "crossref.org",
    "api.crossref.org",
    "chatgpt.com",
];

/// `true` only for safe `https://` links pointing to trusted academic / reference hosts
/// so `target="_blank"` source links can open externally while WebView2 navigation stays
/// strictly pinned to the loopback server.
pub fn is_allowed_external_source_url(url: &str) -> bool {
    if url.is_empty()
        || url.len() > 2048
        || url.chars().any(|c| c.is_ascii_control() || c.is_ascii_whitespace() || c == '\\')
    {
        return false;
    }
    let Some(after_scheme) = url.strip_prefix("https://") else {
        return false;
    };
    let end_auth = after_scheme.find(['/', '?', '#']).unwrap_or(after_scheme.len());
    let authority = &after_scheme[..end_auth];
    if authority.is_empty() || authority.contains('@') || authority.contains(':') {
        return false;
    }
    let host = authority.to_ascii_lowercase();
    ALLOWED_EXTERNAL_HOSTS.contains(&host.as_str())
}

/// Allow session exports and project bibliography downloads from this server.
pub fn is_allowed_export(url: &str, base_url: &str) -> bool {
    let Some(rest) = url.strip_prefix(base_url) else {
        return false;
    };
    let path = if let Some(session) = rest.strip_prefix("/api/sessions/") {
        session.split(['?', '#']).next().unwrap_or_default()
    } else if let Some(project) = rest
        .strip_prefix("/api/projects/")
        .and_then(|path| path.strip_suffix("?format=bib").or_else(|| path.strip_suffix("?format=backup")))
    {
        project
    } else {
        return false;
    };
    let Some(id) = path.strip_suffix("/export") else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 128
        && !matches!(id, "." | "..")
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// The user's Downloads folder, from `USERPROFILE` (no known-folder lookup, so a moved
/// Downloads folder is not followed).
pub fn downloads_dir(userprofile: Option<std::ffi::OsString>) -> Option<PathBuf> {
    userprofile.filter(|v| !v.is_empty()).map(|p| PathBuf::from(p).join("Downloads"))
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A plain Windows file name from whatever name WebView2 suggested: no directories, no
/// characters Windows forbids, no reserved device names, never empty.
pub fn sanitize_file_name(suggested: &str) -> String {
    let last = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    let mut name = cleaned.trim_start().trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() {
        return "re-search-export".to_string();
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    name
}

/// `dir/name`, or `dir/stem (1).ext`, `dir/stem (2).ext`, ... : the first that does not exist
/// yet, so an existing file is never overwritten. `None` if no free name is found.
pub fn unique_download_path(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let free = |p: &Path| std::fs::symlink_metadata(p).is_err();
    let first = dir.join(file_name);
    if free(&first) {
        return Some(first);
    }
    let (stem, ext) = match file_name.rfind('.') {
        Some(i) if i > 0 => (&file_name[..i], &file_name[i..]),
        _ => (file_name, ""),
    };
    (1..10_000u32)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| free(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:43123";

    #[test]
    fn navigation_is_limited_to_this_server_and_about_blank() {
        for ok in [
            "about:blank",
            "http://127.0.0.1:43123",
            "http://127.0.0.1:43123/",
            "http://127.0.0.1:43123/index.html",
            "http://127.0.0.1:43123/?x=1",
            "http://127.0.0.1:43123/#frag",
            "http://127.0.0.1:43123/api/sessions/s-1/export?format=zip",
        ] {
            assert!(is_allowed_navigation(ok, BASE), "{ok}");
        }
        for bad in [
            "http://127.0.0.1:431230/",
            "http://127.0.0.1:43124/",
            "http://127.0.0.1:43123.evil.example/",
            "http://127.0.0.1.evil.example:43123/",
            "http://127.0.0.1:43123@evil.example/",
            "http://localhost:43123/",
            "https://127.0.0.1:43123/",
            "http://127.0.0.1/",
            "https://arxiv.org/abs/1234.5678",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "data:text/html,hi",
            "about:blank#x",
            "",
        ] {
            assert!(!is_allowed_navigation(bad, BASE), "{bad}");
        }
    }

    #[test]
    fn only_session_exports_and_project_bibliographies_may_download() {
        for ok in [
            "http://127.0.0.1:43123/api/sessions/s-0123abcd/export",
            "http://127.0.0.1:43123/api/sessions/s-0123abcd/export?format=zip",
            "http://127.0.0.1:43123/api/sessions/s_1.fork-2/export?format=tex",
            "http://127.0.0.1:43123/api/projects/p-0123abcd/export?format=bib",
            "http://127.0.0.1:43123/api/projects/p-0123abcd/export?format=backup",
        ] {
            assert!(is_allowed_export(ok, BASE), "{ok}");
        }
        for bad in [
            "http://127.0.0.1.evil.example/api/sessions/s-1/export",
            "http://127.0.0.1.evil.example:43123/api/sessions/s-1/export",
            "http://127.0.0.1:43124/api/sessions/s-1/export",
            "http://127.0.0.1:431230/api/sessions/s-1/export",
            "http://127.0.0.1:43123@evil.example/api/sessions/s-1/export",
            "http://localhost:43123/api/sessions/s-1/export",
            "https://127.0.0.1:43123/api/sessions/s-1/export",
            "http://127.0.0.1:43123/api/sessions//export",
            "http://127.0.0.1:43123/api/sessions/../export",
            "http://127.0.0.1:43123/api/sessions/s-1/x/export",
            "http://127.0.0.1:43123/api/sessions/S-UPPER/export",
            "http://127.0.0.1:43123/api/sessions/s-1/events",
            "http://127.0.0.1:43123/api/sessions/s-1/export/more",
            "http://127.0.0.1:43123/api/projects/../export?format=bib",
            "http://127.0.0.1:43123/api/projects/p-1/export",
            "http://127.0.0.1:43123/api/projects/p-1/export?format=zip",
            "http://127.0.0.1:43123/api/projects/p-1/sources?format=bib",
            "http://127.0.0.1:43124/api/projects/p-1/export?format=bib",
            "http://127.0.0.1:43123/assets/app.js",
            "http://127.0.0.1:43123/",
            "https://example.com/api/sessions/s-1/export",
        ] {
            assert!(!is_allowed_export(bad, BASE), "{bad}");
        }
    }

    #[test]
    fn downloads_folder_comes_from_userprofile() {
        let d = downloads_dir(Some("C:\\Users\\someone".into())).unwrap();
        assert_eq!(d, PathBuf::from("C:\\Users\\someone").join("Downloads"));
        assert_eq!(downloads_dir(None), None);
        assert_eq!(downloads_dir(Some("".into())), None);
    }

    #[test]
    fn suggested_names_are_made_safe() {
        assert_eq!(sanitize_file_name("s-1.zip"), "s-1.zip");
        assert_eq!(sanitize_file_name("C:\\Users\\x\\Downloads\\s-1.tex"), "s-1.tex");
        assert_eq!(sanitize_file_name("../../evil.html"), "evil.html");
        assert_eq!(sanitize_file_name("a<b>c:d\"e|f?g*h.zip"), "a_b_c_d_e_f_g_h.zip");
        assert_eq!(sanitize_file_name("name. . "), "name");
        assert_eq!(sanitize_file_name(""), "re-search-export");
        assert_eq!(sanitize_file_name("..."), "re-search-export");
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("nul"), "_nul");
    }

    #[test]
    fn downloads_never_overwrite_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(unique_download_path(dir, "proof.zip").unwrap(), dir.join("proof.zip"));

        std::fs::write(dir.join("proof.zip"), "old").unwrap();
        assert_eq!(unique_download_path(dir, "proof.zip").unwrap(), dir.join("proof (1).zip"));

        std::fs::write(dir.join("proof (1).zip"), "old").unwrap();
        assert_eq!(unique_download_path(dir, "proof.zip").unwrap(), dir.join("proof (2).zip"));

        std::fs::write(dir.join("README"), "old").unwrap();
        assert_eq!(unique_download_path(dir, "README").unwrap(), dir.join("README (1)"));

        std::fs::create_dir(dir.join("folder.tex")).unwrap();
        assert_eq!(unique_download_path(dir, "folder.tex").unwrap(), dir.join("folder (1).tex"));
        assert_eq!(std::fs::read_to_string(dir.join("proof.zip")).unwrap(), "old");
    }

    #[test]
    fn allowed_external_source_urls_are_strictly_https_and_allowlisted() {
        for ok in [
            "https://arxiv.org/abs/2301.12345",
            "https://arxiv.org/pdf/2301.12345",
            "https://doi.org/10.1103/PhysRevD.100.012345",
            "https://inspirehep.net/literature/123456",
            "https://www.semanticscholar.org/paper/abc",
            "https://ARXIV.ORG/abs/hep-th/9901001",
            "https://chatgpt.com/settings/usage",
        ] {
            assert!(is_allowed_external_source_url(ok), "{ok}");
        }
        for bad in [
            "",
            "http://arxiv.org/abs/2301.12345",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "https://evil.example/abs/2301.12345",
            "https://arxiv.org.evil.example/abs/1",
            "https://user@arxiv.org/abs/1",
            "https://arxiv.org:8443/abs/1",
            "https://arxiv.org/abs/1\n",
            "https://arxiv.org/abs/1\\2",
            "http://127.0.0.1:43123/",
        ] {
            assert!(!is_allowed_external_source_url(bad), "{bad}");
        }
    }
}
