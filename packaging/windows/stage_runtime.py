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
PACKAGES = {
    "numpy": "2.5.5",
    "sympy": "1.14.0",
    "mpmath": "1.3.0",
    "scipy": "1.18.1",
    "matplotlib": "3.11.2",
    "contourpy": "1.4.0",
    "cycler": "0.12.1",
    "fonttools": "4.69.1",
    "kiwisolver": "1.5.1",
    "packaging": "26.4",
    "pillow": "13.0.0",
    "pyparsing": "3.3.3",
    "python-dateutil": "4.0.0",
    "tzdata": "2026.6",
}
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
    "scipy": (
        "https://files.pythonhosted.org/packages/52/94/d73da0d28f16c45bb9b0a5691b91610b0275c5ef0eb5e43c87cf2dc1bf31/scipy-1.18.1-cp314-cp314-win_amd64.whl",
        "78a0d7c918e74a232394117160e7e3db503377572a45bcef8826e4ab8a35feba",
    ),
    "matplotlib": (
        "https://files.pythonhosted.org/packages/ea/61/4315ef0d63937fe56f451887b4a0be3b8e8b22cf1d92a8d157caac54570d/matplotlib-3.11.2-cp314-cp314-win_amd64.whl",
        "ecea603dd2fbf8242fd31a305a8b12a4ece2de28096870c65fdd0d1e35b8d9a6",
    ),
    "contourpy": (
        "https://files.pythonhosted.org/packages/15/41/3df8cc14bb572c8b447f9dfbc876703f0f2f2896058285b5c87a1d4a638d/contourpy-1.4.0-cp314-cp314-win_amd64.whl",
        "181bea01bc742734ae672fa00c717d855dd35e1f029536406538c52e7b0cd73d",
    ),
    "cycler": (
        "https://files.pythonhosted.org/packages/e7/05/c19819d5e3d95294a6f5947fb9b9629efb316b96de511b418c53d245aae6/cycler-0.12.1-py3-none-any.whl",
        "85cef7cff222d8644161529808465972e51340599459b8ac3ccbac5a854e0d30",
    ),
    "fonttools": (
        "https://files.pythonhosted.org/packages/18/63/0057e999a328987a9be98418d2c9e8dbffafe79bd3237976b59dab60e046/fonttools-4.69.1-cp314-cp314-win_amd64.whl",
        "84f2d9610b216bd13d2b9a2a26f0e7d31791f4564a7d5a8bc3048919ec2b678a",
    ),
    "kiwisolver": (
        "https://files.pythonhosted.org/packages/4e/0e/49522e1ab5788cbaf63a26fbd3b851f9028616828c961b8a31b35cb96df8/kiwisolver-1.5.1-cp314-cp314-win_amd64.whl",
        "f4167e87b397f273dc2356fcf1eaf50a6bac51e6105f45103ef7129c8efb0255",
    ),
    "packaging": (
        "https://files.pythonhosted.org/packages/ba/7b/5504f91c9c2733f693d7759e50ede5d27119d4da7e5836c46d3b5ff81d56/packaging-26.4-py3-none-any.whl",
        "81383611a09c1bc0b08b76e0fd5829d7c6824c65b27a6aee339da133f4233ea2",
    ),
    "pillow": (
        "https://files.pythonhosted.org/packages/50/e2/7ad17747517e4c813dca78257d6bb593466f8231baf1384bc98d92745edb/pillow-13.0.0-cp314-cp314-win_amd64.whl",
        "f7cbb7a64e7ec664775195e4f81923bc8a93d9451b963f844e59118f63147a7a",
    ),
    "pyparsing": (
        "https://files.pythonhosted.org/packages/38/bb/d215ee7c73b61497b28a5503f9f53523f294fcc936762b7caf90e0c1c2b5/pyparsing-3.3.3-py3-none-any.whl",
        "ece8c00a69cf01b45d0b1dedabb469c90d8caf996d4fda40f147627a122849a4",
    ),
    "python-dateutil": (
        "https://files.pythonhosted.org/packages/2f/a9/52b89563d154433d79b51ed1eb4842bf7334cb2696891a7fe135618853d7/python_dateutil-4.0.0-py3-none-any.whl",
        "a5e246f79eb80475e691bf93862cead98decd8a498cf53ba0516fcdc0f50dc1c",
    ),
    "tzdata": (
        "https://files.pythonhosted.org/packages/0a/66/9a06c2227fedfd4919f0eaaa8b94377d9730882a7e4f95823c9a331ab5e1/tzdata-2026.6-py2.py3-none-any.whl",
        "0a10eec6e13ef16b627fabed9a5f2ba61a62b1c67f327305f4e7e412a70b4aec",
    ),
}

PACKAGE_TOPS = {
    "numpy": {"numpy", "numpy.libs", f"numpy-{PACKAGES['numpy']}.dist-info"},
    "sympy": {"sympy", f"sympy-{PACKAGES['sympy']}.dist-info"},
    "mpmath": {"mpmath", f"mpmath-{PACKAGES['mpmath']}.dist-info"},
    "scipy": {"scipy", "scipy.libs", f"scipy-{PACKAGES['scipy']}.dist-info"},
    "matplotlib": {"matplotlib", "mpl_toolkits", "pylab.py", f"matplotlib-{PACKAGES['matplotlib']}.dist-info"},
    "contourpy": {"contourpy", f"contourpy-{PACKAGES['contourpy']}.dist-info"},
    "cycler": {"cycler", f"cycler-{PACKAGES['cycler']}.dist-info"},
    "fonttools": {"fontTools", f"fonttools-{PACKAGES['fonttools']}.dist-info"},
    "kiwisolver": {"kiwisolver", f"kiwisolver-{PACKAGES['kiwisolver']}.dist-info"},
    "packaging": {"packaging", f"packaging-{PACKAGES['packaging']}.dist-info"},
    "pillow": {"PIL", f"pillow-{PACKAGES['pillow']}.dist-info"},
    "pyparsing": {"pyparsing", f"pyparsing-{PACKAGES['pyparsing']}.dist-info"},
    "python-dateutil": {"dateutil", f"python_dateutil-{PACKAGES['python-dateutil']}.dist-info"},
    "tzdata": {"tzdata", f"tzdata-{PACKAGES['tzdata']}.dist-info"},
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
    if top not in PACKAGE_TOPS[package]:
        return None
    if not top.endswith((".dist-info", ".libs")) and "tests" in path.parts:
        if not any(word in path.name.upper() for word in ("COPYING", "LICENSE")):
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
        f"NumPy {PACKAGES['numpy']} (BSD-3-Clause/MIT/0BSD/Zlib/CC0-1.0): Lib/site-packages/numpy-{PACKAGES['numpy']}.dist-info/licenses/ "
        "(includes OpenBLAS, LAPACK and GCC Runtime Library Exception notices).\n"
        f"SciPy {PACKAGES['scipy']} (BSD-3-Clause): Lib/site-packages/scipy-{PACKAGES['scipy']}.dist-info/LICENSE.txt "
        "(includes OpenBLAS, LAPACK, GCC, Qhull, DOP and duccfft notices).\n"
        f"Matplotlib {PACKAGES['matplotlib']} (PSF-based Matplotlib License): Lib/site-packages/matplotlib-{PACKAGES['matplotlib']}.dist-info/LICENSE "
        "(includes DejaVu and STIX font notices under matplotlib/mpl-data/fonts/ttf/).\n"
        f"SymPy {PACKAGES['sympy']} (BSD-3-Clause): Lib/site-packages/sympy-{PACKAGES['sympy']}.dist-info/licenses/LICENSE.\n"
        f"mpmath {PACKAGES['mpmath']} (BSD-3-Clause): Lib/site-packages/mpmath-{PACKAGES['mpmath']}.dist-info/LICENSE.\n"
        f"contourpy {PACKAGES['contourpy']} (BSD-3-Clause): Lib/site-packages/contourpy-{PACKAGES['contourpy']}.dist-info/licenses/LICENSE.\n"
        f"cycler {PACKAGES['cycler']} (BSD-3-Clause): Lib/site-packages/cycler-{PACKAGES['cycler']}.dist-info/LICENSE.\n"
        f"fonttools {PACKAGES['fonttools']} (MIT): Lib/site-packages/fonttools-{PACKAGES['fonttools']}.dist-info/licenses/LICENSE.\n"
        f"kiwisolver {PACKAGES['kiwisolver']} (BSD-3-Clause): Lib/site-packages/kiwisolver-{PACKAGES['kiwisolver']}.dist-info/licenses/LICENSE.\n"
        f"packaging {PACKAGES['packaging']} (Apache-2.0 OR BSD-2-Clause): Lib/site-packages/packaging-{PACKAGES['packaging']}.dist-info/licenses/LICENSE.\n"
        f"Pillow {PACKAGES['pillow']} (MIT-CMU): Lib/site-packages/pillow-{PACKAGES['pillow']}.dist-info/licenses/LICENSE "
        "(includes libjpeg-turbo, libpng, zlib, FreeType, libtiff, WebP and Little CMS notices).\n"
        f"pyparsing {PACKAGES['pyparsing']} (MIT): Lib/site-packages/pyparsing-{PACKAGES['pyparsing']}.dist-info/licenses/LICENSE.\n"
        f"python-dateutil {PACKAGES['python-dateutil']} (Apache-2.0 / BSD-3-Clause): Lib/site-packages/python_dateutil-{PACKAGES['python-dateutil']}.dist-info/licenses/LICENSE.\n"
        f"tzdata {PACKAGES['tzdata']} (Apache-2.0): Lib/site-packages/tzdata-{PACKAGES['tzdata']}.dist-info/licenses/LICENSE.\n"
        "Bundling adds an isolated import path and omits development tools, package installers and test suites. "
        "This distribution retains the upstream packages' applicable copyright and license notices.\n\n"
        + "\n".join(f"{name}: {url}\nSHA-256 {digest}" for name, (url, digest) in ARCHIVES.items()) + "\n"
    ).encode("utf-8"))
    required = [
        "python.exe", "python314.dll", "Lib/encodings/__init__.py", "DLLs/_ctypes.pyd", "LICENSE.txt",
        "Lib/site-packages/numpy.libs", f"Lib/site-packages/numpy-{PACKAGES['numpy']}.dist-info/licenses/LICENSE.txt",
        "Lib/site-packages/scipy.libs", f"Lib/site-packages/scipy-{PACKAGES['scipy']}.dist-info/LICENSE.txt",
        f"Lib/site-packages/sympy-{PACKAGES['sympy']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/mpmath-{PACKAGES['mpmath']}.dist-info/LICENSE",
        f"Lib/site-packages/matplotlib-{PACKAGES['matplotlib']}.dist-info/LICENSE",
        "Lib/site-packages/matplotlib/mpl-data/fonts/ttf/LICENSE_DEJAVU",
        "Lib/site-packages/mpl_toolkits", "Lib/site-packages/pylab.py",
        f"Lib/site-packages/contourpy-{PACKAGES['contourpy']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/cycler-{PACKAGES['cycler']}.dist-info/LICENSE",
        f"Lib/site-packages/fonttools-{PACKAGES['fonttools']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/kiwisolver-{PACKAGES['kiwisolver']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/packaging-{PACKAGES['packaging']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/pillow-{PACKAGES['pillow']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/pyparsing-{PACKAGES['pyparsing']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/python_dateutil-{PACKAGES['python-dateutil']}.dist-info/licenses/LICENSE",
        f"Lib/site-packages/tzdata-{PACKAGES['tzdata']}.dist-info/licenses/LICENSE",
    ]
    for name in required:
        if not (dest / name).exists():
            raise ValueError(f"Runtime is missing {name}")


SMOKE = """
import importlib.metadata, json, math, os, runpy, sys, tempfile
from pathlib import Path
import numpy, mpmath, sympy
assert sys.flags.isolated and sys.maxsize > 2**32
worker = runpy.run_path(sys.argv[1])
namespace = worker['make_namespace']()
versions = worker['runtime_environment']()
assert versions['python'] == sys.argv[2]
expected = json.loads(sys.argv[3])
assert all(versions[k] == expected[k] for k in ('numpy', 'sympy', 'mpmath'))
assert all(importlib.metadata.version(k) == v for k, v in expected.items())
assert 'scipy' not in sys.modules and 'matplotlib' not in sys.modules
assert numpy.allclose(numpy.linalg.eigvalsh([[2., 1.], [1., 2.]]), [1., 3.])
x = sympy.Symbol('x', real=True)
assert namespace['check_symbolic_identity']((x+1)**2, x*x+2*x+1)['status'] == 'symbolic_agreement'
assert all(callable(namespace[key]) for key in ('check_convergence', 'commutator'))
assert mpmath.almosteq(mpmath.sqrt(2)**2, 2)
from scipy import integrate
quad_val, quad_err = integrate.quad(lambda t: t * math.sin(t), 0.0, math.pi)
assert abs(quad_val - math.pi) < 1e-12 and quad_err < 1e-12
with tempfile.TemporaryDirectory() as tmp:
    os.environ['MPLBACKEND'] = 'Agg'
    os.environ['MPLCONFIGDIR'] = str(Path(tmp) / 'mpl')
    import matplotlib
    import matplotlib.pyplot as plt
    assert matplotlib.get_backend().lower() == 'agg'
    fig, ax = plt.subplots()
    ax.plot([0.0, 1.0, 2.0], [0.0, 1.0, 4.0], label='x^2')
    ax.set_title('Offline Agg Smoke')
    ax.legend()
    out = Path(tmp) / 'smoke.png'
    fig.savefig(out)
    plt.close(fig)
    png = out.read_bytes()
    assert png.startswith(b'\\x89PNG\\r\\n\\x1a\\n') and len(png) > 1000
print('Offline Windows worker math check passed:', {k: importlib.metadata.version(k) for k in ('numpy','sympy','mpmath','scipy','matplotlib')})
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
