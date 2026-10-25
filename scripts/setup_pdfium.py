#!/usr/bin/env python3
"""Install a pinned, checksum-verified, non-JavaScript PDFium runtime.

Run once after cloning. Release packages should place the library and the
accompanying licenses beside caret. No download occurs when viewing a file.
"""
import argparse
import hashlib
import io
from pathlib import Path
import platform
import shutil
import tarfile
import tempfile
import urllib.request

VERSION = "chromium/8111"
HASHES = {
    "win-x64": "3452d11031050ec65e57f180215b57b9e6d465e3fbedafa8a77ad90885c7d23e",
    "win-arm64": "99ec8b76d5ac30602a8488da745397393ff6cddc824ef36f398146f6b4f76d4e",
    "linux-x64": "bdc60126399f39fe768566a18d4be055a7b3681bc9e248e8a6656b4f6cae0aa2",
    "linux-arm64": "042de247138f96341534cccf24ed21c2550a7245b936bb4a60d2340120000f05",
    "mac-x64": "e7fece4693f65a7e47f84fa38099068d573d4dcbd46b1a3c8d2c26581c308466",
    "mac-arm64": "46c674298c60123f1195c2a03c65ee08c4783c84874ce4f5b3a054abb3ecaf30",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--install-dir", type=Path, help="Also copy runtime and licenses beside an installed caret binary")
    args = parser.parse_args()
    os_name = {"Windows": "win", "Linux": "linux", "Darwin": "mac"}.get(platform.system())
    arch = {"AMD64": "x64", "x86_64": "x64", "aarch64": "arm64", "arm64": "arm64", "ARM64": "arm64"}.get(platform.machine())
    key = f"{os_name}-{arch}"
    if key not in HASHES:
        parser.error(f"No PDFium runtime is configured for {platform.system()} {platform.machine()}")
    name = f"pdfium-{key}.tgz"
    url = f"https://github.com/bblanchon/pdfium-binaries/releases/download/{VERSION}/{name}"
    print(f"Downloading PDFium {VERSION} ({key})...")
    with urllib.request.urlopen(url, timeout=30) as response:
        data = response.read(32 * 1024 * 1024 + 1)
    if len(data) > 32 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != HASHES[key]:
        raise SystemExit("PDFium archive checksum/size validation failed")
    root = Path(__file__).resolve().parent.parent
    runtime = root / "runtime" / "pdfium"
    runtime.parent.mkdir(parents=True, exist_ok=True)
    # The only recursively cleaned directory is this exclusively created staging
    # directory. Never remove an existing runtime or any caller-owned files.
    with tempfile.TemporaryDirectory(prefix="pdfium-", dir=runtime.parent) as staging:
        staging = Path(staging)
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            archive.extractall(staging, filter="data")
        runtime.mkdir(exist_ok=True)
        shutil.copytree(staging, runtime, dirs_exist_ok=True)
    library = runtime / ("bin/pdfium.dll" if os_name == "win" else f"lib/libpdfium.{'dylib' if os_name == 'mac' else 'so'}")
    if not library.is_file():
        raise SystemExit(f"Verified archive did not contain {library.name}")
    if args.install_dir:
        install = args.install_dir.resolve()
        install.mkdir(parents=True, exist_ok=True)
        shutil.copy2(library, install / library.name)
        # Preserve all third-party notices supplied by the runtime distributor.
        licenses = runtime / "licenses"
        if licenses.is_dir():
            shutil.copytree(licenses, install / "pdfium-licenses", dirs_exist_ok=True)
        for notice in ("LICENSE", "LICENSE.txt", "NOTICE", "VERSION"):
            if (runtime / notice).is_file():
                shutil.copy2(runtime / notice, install / f"pdfium-{notice}")
    print(f"PDFium ready: {library}")


if __name__ == "__main__":
    main()
