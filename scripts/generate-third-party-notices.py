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

ROOT = Path(__file__).resolve().parent.parent
EXTRAS = ROOT / "scripts" / "third-party"
LEGACY_LICENSES = ROOT / "licenses" / "legacy"
RELEASE_PACKAGES = {"desktop-app", "cli", "mcp-server"}
LEGAL_NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense|authors|patents|third.party)([_.-]|$)", re.I)
WEBVIEW2_CRATE = "0.39.1"
WEBVIEW2_SDK = "1.0.3800.47"
WEBVIEW2_HASHES = {
    "x86_64-pc-windows-msvc": ("x64/WebView2LoaderStatic.lib", "89c6b872783b8f6c3cedbff618adb42082d455c615453ae10cfc753f1e8f25d8"),
    "x86_64-pc-windows-gnu": ("x64/WebView2Loader.dll", "86545b66cdb0603bc26b626fb9ad610cb6e71f28d468f5ea66df23b03dda96d5"),
}
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
    if webview is None or webview["version"] != WEBVIEW2_CRATE:
        raise ValueError("review WebView2 SDK redistribution files for the locked binding version")
    relative, expected = WEBVIEW2_HASHES[target]
    loader = Path(webview["manifest_path"]).parent / relative
    if digest(loader) != expected:
        raise ValueError(f"the locked WebView2 Loader does not match SDK {WEBVIEW2_SDK}")
    for suffix in ("LICENSE", "NOTICE"):
        copy(EXTRAS / f"WebView2Loader-{suffix}.txt", dest / "WebView2Loader")
    vendor = ROOT / "crates" / "desktop-server" / "vendor" / "katex"
    if b'version:"0.19.1"' not in (vendor / "katex.min.js").read_bytes():
        raise ValueError("review KaTeX licenses and font notices for the vendored version")
    copy(vendor / "LICENSE", dest / "KaTeX", "KaTeX-MIT.txt")
    copy(EXTRAS / "KaTeX-Fonts-OFL.txt", dest / "KaTeX")
    for filename in ("NSIS-Zlib-License.txt", "zlib-License.txt"):
        copy(EXTRAS / filename, dest / "NSIS")
    return relative, expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=sorted(WEBVIEW2_HASHES), required=True)
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
        "reSearch: GNU General Public License, version 3 only (see ../LICENSE).",
        f"Project source archive: {source_url}",
        "Build and installation scripts are in the archive. Run cargo fetch --locked",
        "in the source tree to retrieve the dependency versions and checksums pinned in Cargo.lock.",
        "Earlier MIT and Apache-2.0 grants are recorded in legacy/LICENSE-MIT and legacy/LICENSE-APACHE.",
        "",
        f"Target: {args.target}",
        f"Cargo.lock SHA-256: {digest(ROOT / 'Cargo.lock')}",
        f"Rust toolchain: {rustc}",
        "",
        "The standard library copyright report is in Rust-standard-library/.",
        "The KaTeX 0.19.1 JavaScript, CSS, and fonts are covered in KaTeX/.",
        "Notices for the NSIS installer stub and zlib are in NSIS/.",
        f"Microsoft.Web.WebView2 SDK {WEBVIEW2_SDK} notices are in WebView2Loader/.",
        f"Loader: {loader} (SHA-256: {checksum})",
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
