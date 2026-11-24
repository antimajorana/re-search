# Windows installer

`reSearch-Setup-Windows-x64.exe` installs reSearch for the current Windows user. Open **reSearch** from the Start menu. The native window requires the Microsoft Edge WebView2 Runtime. Installer and uninstaller both accept `/S` for a silent run. With the bundled Python runtime, review the Python license before passing `/S /ACCEPT_PYTHON_LICENSE` to the installer.

The installer uses `%LOCALAPPDATA%\Programs\reSearch`. The desktop app, `re-search.exe`, `core.exe`, and the Python worker scripts live together so the usual discovery rules apply. The CLI is available from its installation directory. Bundled builds also include a private Python runtime and its numerical libraries. Lean and optional model-provider CLIs are configured separately.

Close reSearch and its command-line processes before upgrading or uninstalling. Upgrades replace the packaged files and retain `re-search-app.json` beside the executables. Uninstall removes the packaged files, Start menu shortcut and installed-apps entry. Research sessions and other user data stay in `%LOCALAPPDATA%\re-search` by default. A user-provided config file stays in the installation directory.

## License and source

The installed `LICENSE` contains GPLv3 for reSearch. The `licenses/legacy/` directory preserves notices from earlier releases. `licenses/THIRD-PARTY-NOTICES.txt` identifies the exact project source archive for an official installer and the source packages for its Rust dependencies. The bundled Python distribution and numerical libraries carry their own notices under `runtime/`.

### Windows binary release review

Version 0.1.4 was released under the earlier license terms. Current Windows builds compile the runtime loader from the project's source archive and locate the separately installed Edge WebView2 Runtime. The release job checks the resolved loader source, the native link map, and successful startup of the installed WebView2 application. [Loader source and provenance](../../vendor/webview2-com-sys/README.md)

## Build

Use NSIS 3.11 or later. Build the Rust binaries in release mode for a single Windows x64 target. Stage the following files and directories at an absolute path.

```text
re-search-app.exe
re-search.exe
core.exe
LICENSE
licenses/
python/
  worker.py
  solver_calc.py
  numerical_validation.py
  symbolic_validation.py
```

For a bundled Python build, stage the full `runtime/` tree, including package notices, and pass `-DRE_SEARCH_BUNDLED_PYTHON`.

Generate `licenses/` from the same checkout and toolchain as the binaries. In GitHub Actions the generator uses the checked-out commit and workflow repository. Outside Actions, or when packaging local modifications, pass `--source-url` with the URL of the exact source archive for those binaries. [Notice generator](../../scripts/third-party/SOURCES.md)

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
  -DRE_SEARCH_BUNDLED_PYTHON \
  packaging/windows/re-search.nsi
```

In PowerShell, pass the corresponding absolute Windows paths as quoted arguments. Install and uninstall in a Windows test profile before publishing.
