"""Copy PDF.js's published browser renderer, offline resources and licenses into the desktop bundle."""
import base64
import hashlib
import io
from pathlib import Path
import tarfile
import urllib.request
import zipfile

VERSION = "6.5.438"
SHA512 = "vg8Pw1x+AFKy4AMDvhuWl5cOYV8d698UoEANoFFhc79hqXDnZQqnOXoxw3HG907WHOg1JYOvLhP756boddFxpA=="
ROOT = Path(__file__).resolve().parents[1]


def main():
    url = f"https://registry.npmjs.org/pdfjs-dist/-/pdfjs-dist-{VERSION}.tgz"
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    if hashlib.sha512(data).digest() != base64.b64decode(SHA512):
        raise ValueError("PDF.js package checksum does not match the pinned release")
    destination = ROOT / "crates/desktop-server/vendor/pdfjs" / "renderer.zip"
    destination.parent.mkdir(exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data)) as source, zipfile.ZipFile(destination, "w") as bundle:
        for member in sorted(source.getmembers(), key=lambda item: item.name):
            name = member.name.removeprefix("package/")
            if not member.isfile() or "quickjs" in name.lower():
                continue
            if name in ("build/pdf.min.mjs", "build/pdf.worker.min.mjs", "LICENSE") or name.startswith(
                ("cmaps/", "standard_fonts/", "iccs/", "wasm/")
            ):
                # Stored entries can be served as slices of the embedded binary without inflation.
                bundle.writestr(zipfile.ZipInfo(name), source.extractfile(member).read())


if __name__ == "__main__":
    main()
