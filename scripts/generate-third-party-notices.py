#!/usr/bin/env python3
"""Collect redistribution notices for the locked Windows installer build."""

import argparse
import hashlib
from itertools import islice
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parent.parent
EXTRAS = ROOT / "scripts" / "third-party"
LEGACY_LICENSES = ROOT / "licenses" / "legacy"
RELEASE_PACKAGES = {"desktop-app", "cli", "mcp-server"}
LEGAL_NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense|authors|patents|third.party)([_.-]|$)", re.I)
WEBVIEW2_CRATE = "0.39.1"
WINDOWS_TARGETS = ("x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu")
# crates.io archives omit these license files. Match exact locked versions so
# that an upgrade cannot accidentally reuse this attribution.
WEBVIEW2_BINDINGS = {
    ("webview2-com", "0.39.1"),
    ("webview2-com-sys", "0.39.1"),
    ("webview2-com-macros", "0.8.1"),
}
MIT_METADATA_ONLY = {
    ("adobe-cmap-parser", "0.4.1"),
    ("pdf-extract", "0.12.1"),
    ("type1-encoding-parser", "0.1.1"),
}


def capture(*command):
    return subprocess.check_output(command, cwd=ROOT, encoding="utf-8")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rust_packages(cargo, target):
    metadata = json.loads(capture(cargo, "metadata", "--locked", "--offline",
                                  "--format-version", "1", "--filter-platform", target))
    # cargo metadata describes the full workspace resolve. Use cargo tree for
    # the normal edges selected for these three release packages and target.
    command = [cargo, "tree", "--locked", "--offline", "--target", target,
               "--edges", "normal", "--format", "{p}", "--prefix", "none"]
    for name in sorted(RELEASE_PACKAGES):
        command.extend(("-p", name))
    selected = set()
    for line in capture(*command).splitlines():
        if line.strip():
            name, version, *_ = line.split()
            selected.add((name, version.removeprefix("v")))
    packages = []
    workspace = set(metadata["workspace_members"])
    by_version = {}
    for package in metadata["packages"]:
        key = (package["name"], package["version"])
        by_version.setdefault(key, []).append(package)
    for key in sorted(selected):
        matches = by_version.get(key, [])
        if len(matches) != 1:
            raise ValueError(f"could not identify exactly one Cargo manifest for {key}")
        package, = matches
        if package["id"] not in workspace:
            packages.append(package)
    return packages


def copy(path, into, name=None):
    into.mkdir(parents=True, exist_ok=True)
    return shutil.copyfile(path, into / (name or path.name))


def package_notices(package, dest):
    name, version = key = (package["name"], package["version"])
    source = Path(package["manifest_path"]).parent
    dest.mkdir(parents=True, exist_ok=True)
    files = {path for path in source.iterdir() if path.is_file() and LEGAL_NAME.match(path.name)}
    if package["license_file"]:
        files.add(source / package["license_file"])
    if (source / "LICENSES").is_dir():
        files.update(path for path in (source / "LICENSES").rglob("*") if path.is_file())
    if key in WEBVIEW2_BINDINGS:
        files.add(EXTRAS / "webview2-rs-MIT.txt")
    for path in sorted(files):
        relative = path.relative_to(source) if path.is_relative_to(source) else Path(path.name)
        copy(path, dest / relative.parent, relative.name)
    if not files:
        if key not in MIT_METADATA_ONLY or package["license"] != "MIT":
            raise ValueError(f"no license text found for {name} {version}")
        # These releases declare MIT and contain no separate copyright notice.
        # Provide the permission and disclaimer without inventing a year.
        mit = (LEGACY_LICENSES / "LICENSE-MIT").read_text(encoding="utf-8")
        grant = mit[mit.index("Permission is hereby granted"):]
        authors = ", ".join(package["authors"])
        (dest / "MIT.txt").write_bytes(
            f"{name} {version}\nAuthors listed in the crate manifest: {authors}\n"
            f"{package['repository']}\n\nMIT License\n\n{grant}".encode("utf-8")
        )
    if key == ("pdf-extract", "0.12.1"):
        for filename in ("Adobe-Glyph-List-LICENSE.txt", "PDFBox-NOTICE.txt"):
            copy(EXTRAS / filename, dest)
        copy(LEGACY_LICENSES / "LICENSE-APACHE", dest, "PDFBox-Apache-2.0.txt")
    if name == "libsqlite3-sys":
        # The bundled SQLite source identifies SQLite as public domain. The
        # SQLCipher license in the same archive applies to a separate build.
        with (source / "sqlite3" / "sqlite3.c").open(encoding="utf-8") as amalgamation:
            header = "".join(islice(amalgamation, 43))
        if "disclaims copyright" not in header:
            raise ValueError("could not locate the notice in the bundled SQLite source")
        (dest / "SQLite-public-domain.txt").write_bytes(header.encode("utf-8"))


def native_notices(packages, target, dest):
    webview = next((p for p in packages if p["name"] == "webview2-com-sys"), None)
    local = ROOT / "vendor" / "webview2-com-sys"
    if (not webview or webview["version"] != WEBVIEW2_CRATE or webview["source"] is not None
            or Path(webview["manifest_path"]).resolve() != (local / "Cargo.toml").resolve()):
        raise ValueError("the release must resolve the source-built WebView2 loader from this checkout")
    if any(p.suffix.lower() in {".lib", ".dll", ".a", ".obj"} for p in local.rglob("*")) or (local / "build.rs").exists():
        raise ValueError("unexpected binary input or build script in the WebView2 loader")
    for name in ("LICENSE-BINDINGS", "LICENSE-LOADER", "README.md"):
        copy(local / name, dest / "WebView2Loader")
    vendor = ROOT / "crates" / "desktop-server" / "vendor" / "katex"
    if b'version:"0.19.1"' not in (vendor / "katex.min.js").read_bytes():
        raise ValueError("review KaTeX licenses and font notices for the vendored version")
    copy(vendor / "LICENSE", dest / "KaTeX", "KaTeX-MIT.txt")
    copy(EXTRAS / "KaTeX-Fonts-OFL.txt", dest / "KaTeX")
    with zipfile.ZipFile(ROOT / "crates/desktop-server/vendor/pdfjs/renderer.zip") as renderer:
        for name in renderer.namelist():
            if LEGAL_NAME.match(Path(name).name):
                target_notice = dest / "PDF.js" / Path(name.replace("/", "-"))
                target_notice.parent.mkdir(parents=True, exist_ok=True)
                target_notice.write_bytes(renderer.read(name))
    for filename in ("NSIS-Zlib-License.txt", "zlib-License.txt"):
        copy(EXTRAS / filename, dest / "NSIS")
    return "vendor/webview2-com-sys/src/loader.rs", digest(local / "src" / "loader.rs")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=WINDOWS_TARGETS, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--source-url", help="Published source archive for this build, including downstream changes")
    parser.add_argument("--cargo", default=os.getenv("CARGO", "cargo"))
    parser.add_argument("--rustc", default=os.getenv("RUSTC", "rustc"))
    args = parser.parse_args()

    source_url = args.source_url
    if not source_url:
        server = os.getenv("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
        repository = os.getenv("GITHUB_REPOSITORY")
        if not repository:
            parser.error("supply --source-url for a build outside GitHub Actions")
        revision = capture("git", "rev-parse", "HEAD").strip()
        source_url = f"{server}/{repository}/archive/{revision}.tar.gz"

    packages = rust_packages(args.cargo, args.target)
    dest = args.output_dir
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        copy(LEGACY_LICENSES / name, dest / "legacy")
    loader, checksum = native_notices(packages, args.target, dest)
    rustc = capture(args.rustc, "--version").strip()
    sysroot = Path(capture(args.rustc, "--print", "sysroot").strip())
    rust_doc = sysroot / "share" / "doc" / "rust"
    stdlib = dest / "Rust-standard-library"
    copy(rust_doc / "COPYRIGHT-library.html", stdlib)
    for path in sorted((rust_doc / "licenses").glob("*.txt")):
        copy(path, stdlib / "licenses")
    inventory = [
        "reSearch Windows installer source and third-party notices",
        "",
        "reSearch: GNU Affero General Public License, version 3 only (see ../LICENSE).",
        f"Project source archive: {source_url}",
        "Build and installation scripts are in the archive. Run cargo fetch --locked",
        "in the source tree to retrieve the dependency versions and checksums pinned in Cargo.lock.",
        "Earlier MIT and Apache-2.0 grants are recorded in legacy/LICENSE-MIT and legacy/LICENSE-APACHE.",
        "Versions published under GPL-3.0-only before 2026-11-25 remain available under that license.",
        "",
        f"Target: {args.target}",
        f"Cargo.lock SHA-256: {digest(ROOT / 'Cargo.lock')}",
        f"Rust toolchain: {rustc}",
        "",
        "The standard library copyright report is in Rust-standard-library/.",
        "The KaTeX 0.19.1 JavaScript, CSS, and fonts are covered in KaTeX/.",
        "Mozilla PDF.js 6.5.438, its fonts, character maps and image decoders are covered in PDF.js/.",
        "PDF.js source: https://github.com/mozilla/pdf.js/tree/v6.5.438",
        "Notices for the NSIS installer stub and zlib are in NSIS/.",
        "Source-built WebView2 runtime loader and upstream MIT notices: WebView2Loader/.",
        f"Loader source in project archive: {loader} (SHA-256: {checksum})",
        "",
        "Normal locked Cargo dependencies of re-search-app.exe, re-search.exe, and core.exe",
        "",
    ]
    for package in packages:
        name, version = package["name"], package["version"]
        directory = f"Rust-components/{name}-{version}"
        package_notices(package, dest / directory)
        declared = package["license"] or "see upstream license text"
        location = package["source"] or source_url
        if location == "registry+https://github.com/rust-lang/crates.io-index":
            location = f"https://crates.io/api/v1/crates/{name}/{version}/download"
        inventory.append(f"{name} {version}  |  {declared}  |  {directory}/  |  {location}")
    (dest / "THIRD-PARTY-NOTICES.txt").write_bytes(("\n".join(inventory) + "\n").encode("utf-8"))
    print(f"Collected notices for {len(packages)} locked Rust dependencies and bundled assets.")


if __name__ == "__main__":
    main()
