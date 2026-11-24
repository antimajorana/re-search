"""Build the isolated Windows x64 math runtime from SHA-256-pinned PSF and PyPI archives."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tempfile
from urllib.request import urlopen
from zipfile import ZipFile

PYTHON_VERSION = "3.14.10"
PACKAGES = {"numpy": "2.5.5", "sympy": "1.14.0", "mpmath": "1.3.0"}
ARCHIVES = {
    "python": (
        "https://api.nuget.org/v3-flatcontainer/python/3.14.10/python.3.14.10.nupkg",
        "f54ba67c9951c3d773ea47d2bb095150c037c399f0423a968bcda22809241784",
    ),
    "numpy": (
        "https://files.pythonhosted.org/packages/74/e2/494516771374b43060c1b778b371cba53d576abd3aee1275eecd73896e3b/numpy-2.5.5-cp314-cp314-win_amd64.whl",
        "20145b11776d2f7570cd61351b156a51d3a341822282095baa819d87272bc710",
    ),
    "sympy": (
        "https://files.pythonhosted.org/packages/a2/09/77d55d46fd61b4a135c444fc97158ef34a095e5681d0a6c10b75bf356191/sympy-1.14.0-py3-none-any.whl",
        "e091cc3e99d2141a0ba2847328f5479b05d94a6635cb96148ccb3f34671bd8f5",
    ),
    "mpmath": (
        "https://files.pythonhosted.org/packages/43/e3/7d92a15f894aa0c9c4b49b8ee9ac9850d6e63b03c9c32c0367a13ae62209/mpmath-1.3.0-py3-none-any.whl",
        "a0b2b9fe80bbcd81a6647ff13108738cfb482d481d826cc0e02f5b35e5c88d2c",
    ),
}


def checked_archive(cache, url, digest, offline):
    archive = cache / url.rsplit("/", 1)[1]
    if not archive.is_file():
        if offline:
            raise ValueError(f"Missing offline archive: {archive}")
        with tempfile.NamedTemporaryFile(dir=cache, delete=False) as target:
            part = Path(target.name)
        try:
            with urlopen(url, timeout=90) as response, part.open("wb") as dest:
                shutil.copyfileobj(response, dest)
            if hashlib.sha256(part.read_bytes()).hexdigest() != digest:
                raise ValueError(f"Download checksum mismatch: {archive.name}")
            part.replace(archive)
        finally:
            part.unlink(missing_ok=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
        raise ValueError(f"Cached archive checksum mismatch: {archive}")
    return archive


def target_member(package, name):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name:
        raise ValueError(f"Unexpected archive member: {name}")
    if package == "python":
        if path.parts[0] != "tools" or len(path.parts) < 2:
            return None
        relative = PurePosixPath(*path.parts[1:])
        if relative.parts[0] == "Lib":
            if relative.parts[1:2] in (("site-packages",), ("ensurepip",), ("test",)):
                return None
        elif relative.parts[0] != "DLLs" and relative.name not in {
            "python.exe", "python3.dll", "python314.dll", "vcruntime140.dll", "vcruntime140_1.dll", "LICENSE.txt"
        }:
            return None
        return relative
    # Keep runtime modules, native DLLs and upstream attribution, including embedded library notices.
    top = path.parts[0]
    dist_info = f"{package}-{PACKAGES[package]}.dist-info"
    if top not in {package, dist_info} and not (package == "numpy" and top == "numpy.libs"):
        return None
    if top == package and "tests" in path.parts and not any(word in path.name.upper() for word in ("COPYING", "LICENSE")):
        return None
    return PurePosixPath("Lib/site-packages") / path


def build_runtime(dest, cache, offline):
    for package, (url, digest) in ARCHIVES.items():
        with ZipFile(checked_archive(cache, url, digest, offline)) as archive:
            for entry in archive.infolist():
                if entry.is_dir():
                    continue
                relative = target_member(package, entry.filename)
                if relative is None:
                    continue
                path = dest / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(entry) as src, path.open("xb") as out:
                    shutil.copyfileobj(src, out)

    # An isolated interpreter searches only its installation and the selected packages.
    (dest / "python314._pth").write_bytes(b".\nDLLs\nLib\nLib/site-packages\n")
    (dest / "THIRD-PARTY.txt").write_bytes((
        f"Bundled computation runtime for Windows x64\n"
        f"CPython {PYTHON_VERSION}: Python Software Foundation. See LICENSE.txt, including the Microsoft Distributable Code terms.\n"
        f"NumPy {PACKAGES['numpy']}: Lib/site-packages/numpy-{PACKAGES['numpy']}.dist-info/licenses/ "
        "(includes OpenBLAS, LAPACK and GCC Runtime Library Exception notices).\n"
        f"SymPy {PACKAGES['sympy']}: Lib/site-packages/sympy-{PACKAGES['sympy']}.dist-info/licenses/LICENSE.\n"
        f"mpmath {PACKAGES['mpmath']}: Lib/site-packages/mpmath-{PACKAGES['mpmath']}.dist-info/LICENSE.\n"
        "Bundling adds an isolated import path and omits development tools, package installers and test suites. "
        "This distribution retains the upstream packages' applicable copyright and license notices.\n\n"
        + "\n".join(f"{name}: {url}\nSHA-256 {digest}" for name, (url, digest) in ARCHIVES.items()) + "\n"
    ).encode("utf-8"))
    required = [
        "python.exe", "python314.dll", "Lib/encodings/__init__.py", "DLLs/_ctypes.pyd", "LICENSE.txt",
        "Lib/site-packages/numpy.libs", "Lib/site-packages/numpy-2.5.5.dist-info/licenses/LICENSE.txt",
        "Lib/site-packages/sympy-1.14.0.dist-info/licenses/LICENSE", "Lib/site-packages/mpmath-1.3.0.dist-info/LICENSE",
    ]
    for name in required:
        if not (dest / name).exists():
            raise ValueError(f"Runtime is missing {name}")


SMOKE = """
import json, runpy, sys
from pathlib import Path
import numpy, mpmath, sympy
assert sys.flags.isolated and sys.maxsize > 2**32
worker = runpy.run_path(sys.argv[1])
namespace = worker['make_namespace']()
versions = worker['runtime_environment']()
assert versions['python'] == sys.argv[2]
assert all(versions[k] == v for k, v in json.loads(sys.argv[3]).items())
assert numpy.allclose(numpy.linalg.eigvalsh([[2., 1.], [1., 2.]]), [1., 3.])
x = sympy.Symbol('x', real=True)
assert namespace['check_symbolic_identity']((x+1)**2, x*x+2*x+1)['status'] == 'symbolic_agreement'
assert all(callable(namespace[key]) for key in ('check_convergence', 'commutator'))
assert mpmath.almosteq(mpmath.sqrt(2)**2, 2)
print('Offline Windows worker math check passed:', {k:versions[k] for k in ('python','numpy','sympy','mpmath')})
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New runtime directory next to the packaged executables")
    parser.add_argument("--cache-dir", type=Path, help="Directory of reusable, checksum-verified source archives")
    parser.add_argument("--offline", action="store_true", help="Use cached archives only")
    args = parser.parse_args()
    dest = args.output.resolve()
    if dest.exists() and (not dest.is_dir() or any(dest.iterdir())):
        parser.error(f"Output must be a new or empty directory: {dest}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="re-search-runtime-", dir=dest.parent) as temp:
        cache = args.cache_dir or Path(temp, "downloads")
        cache.mkdir(parents=True, exist_ok=True)
        runtime = Path(temp, "runtime")
        runtime.mkdir()
        build_runtime(runtime, cache, args.offline)
        if os.name == "nt":
            worker = Path(__file__).resolve().parents[2] / "python" / "worker.py"
            subprocess.run([runtime / "python.exe", "-I", "-B", "-c", SMOKE, worker, PYTHON_VERSION, json.dumps(PACKAGES)], check=True)
        for path in sorted(runtime.rglob("*"), reverse=True):
            os.utime(path, (946684800, 946684800))
        if dest.exists():
            dest.rmdir()
        runtime.replace(dest)
    print(f"Staged Windows x64 compute runtime at {dest}")


if __name__ == "__main__":
    main()
