# Windows installer

`reSearch-Setup-Windows-x64.exe` installs reSearch for the current Windows user. Open **reSearch** from the Start menu. The native window requires the Microsoft Edge WebView2 Runtime. Installer and uninstaller both accept `/S` for a silent run. With the bundled Python runtime, review the Python license before passing `/S /ACCEPT_PYTHON_LICENSE` to the installer.

The installer uses `%LOCALAPPDATA%\Programs\reSearch`. The desktop app, `re-search.exe`, `core.exe`, and the Python worker scripts live together so the usual discovery rules apply. The CLI is available from its installation directory. Bundled builds also include a private Python runtime and its numerical libraries. Lean and optional model-provider CLIs are configured separately.

Close reSearch and its command-line processes before upgrading or uninstalling. Upgrades replace the packaged files and retain `re-search-app.json` beside the executables. Uninstall removes the packaged files, Start menu shortcut and installed-apps entry. Research sessions and other user data stay in `%LOCALAPPDATA%\re-search` by default. A user-provided config file stays in the installation directory.

## Build

Use NSIS 3.11 or later. Build the Rust binaries in release mode for a single Windows x64 target. Stage the following files and directories at an absolute path.

```text
re-search-app.exe
re-search.exe
core.exe
LICENSE-APACHE
LICENSE-MIT
licenses/
python/
  worker.py
  solver_calc.py
  numerical_validation.py
  symbolic_validation.py
```

Stage `WebView2Loader.dll` from the matching WebView2 SDK dependency for a GNU build and pass `-DRE_SEARCH_WEBVIEW2_LOADER`. MSVC builds link the loader statically. For a bundled Python build, stage the full `runtime/` tree, including package notices, and pass `-DRE_SEARCH_BUNDLED_PYTHON`.

Build the runtime from the repository root with the Python standard library. The build helper verifies the pinned Python Software Foundation Windows x64 NuGet archive and all three PyPI wheels by SHA-256. Keep the cache to reproduce the runtime offline in a new output directory.

```sh
python packaging/windows/stage_runtime.py --cache-dir /path/to/cache --output /absolute/path/to/stage/runtime
python packaging/windows/stage_runtime.py --offline --cache-dir /path/to/cache --output /another/stage/runtime
```

On Windows the helper runs the worker's symbolic identity check and a NumPy eigenvalue computation with the packaged interpreter. The release smoke test also exercises the installed runtime through AppContainer. The runtime includes the upstream licenses, the Math library distribution metadata, and `THIRD-PARTY.txt`. Retain those files when distributing the installer.

```sh
makensis -WX -DRE_SEARCH_VERSION=0.1.0 \
  -DRE_SEARCH_STAGE=/absolute/path/to/stage \
  -DRE_SEARCH_INSTALLER=/absolute/path/to/reSearch-Setup-Windows-x64.exe \
  -DRE_SEARCH_WEBVIEW2_LOADER -DRE_SEARCH_BUNDLED_PYTHON \
  packaging/windows/re-search.nsi
```

In PowerShell, pass the corresponding absolute Windows paths as quoted arguments. Install and uninstall in a Windows test profile before publishing.
