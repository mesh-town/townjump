#!/usr/bin/env python3
"""Build the townjump .deb (ar of debian-binary, control.tar.gz, data.tar.gz) without dpkg-deb.

deb.py --binary P --arch {amd64,arm64,armhf} --version V-N --out DIR --license P --readme P
       [--keyring P --base-url U]
SOURCE_DATE_EPOCH is required. Exit codes: 0 ok, 2 bad arguments or missing input.
The package holds the binary and two doc files only: no service, no maintainer scripts, no Depends
(the binary is static). With --keyring it also carries the apt source and the key, and only then.
"""
import argparse
import gzip
import hashlib
import io
import os
import re
import sys
import tarfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
NAME = "townjump"
ARCHES = ("amd64", "arm64", "armhf")
MAINTAINER = "Mesh Town <340663027+mesh-town@users.noreply.github.com>"
URL = "https://github.com/mesh-town/townjump"
DESCRIPTION = "Holesail-compatible peer-to-peer tunnels: share a local port and reach it by key."
AR_MAGIC = b"!<arch>\n"
VERSION_RE = re.compile(r"\d+\.\d+\.\d+-[1-9]\d*")


def _tar_gz(entries, sde: int) -> bytes:
    """entries: (name, bytes or None for a directory, mode); GNU format, root owner, sorted by name."""
    bio = io.BytesIO()
    with tarfile.open(fileobj=bio, mode="w", format=tarfile.GNU_FORMAT) as t:
        for name, data, mode in sorted(entries, key=lambda e: e[0]):
            ti = tarfile.TarInfo(name)
            ti.uid = ti.gid = 0
            ti.uname = ti.gname = "root"
            ti.mtime = sde
            ti.mode = mode
            ti.size = 0 if data is None else len(data)
            ti.type = tarfile.DIRTYPE if data is None else tarfile.REGTYPE
            t.addfile(ti, None if data is None else io.BytesIO(data))
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0, filename="") as g:
        g.write(bio.getvalue())
    return out.getvalue()


def _ar_member(name: str, data: bytes, sde: int) -> bytes:
    header = f"{name:<16}{sde:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n"
    return header.encode("ascii") + data + (b"\n" if len(data) % 2 else b"")


def _data_files(binary, license_file, readme, arch, keyring, base_url) -> dict:
    """Package path (no './') -> (bytes, mode)."""
    files = {
        f"usr/bin/{NAME}": (Path(binary).read_bytes(), 0o755),
        f"usr/share/doc/{NAME}/copyright": (Path(license_file).read_bytes(), 0o644),
        f"usr/share/doc/{NAME}/README.md": (Path(readme).read_bytes(), 0o644),
    }
    if keyring is not None:
        text = (HERE / f"{NAME}.sources.in").read_text()
        text = text.replace("@ARCH@", arch).replace("@BASE@", base_url.rstrip("/"))
        files[f"etc/apt/sources.list.d/{NAME}.sources"] = (text.encode(), 0o644)
        files[f"usr/share/keyrings/{NAME}.gpg"] = (Path(keyring).read_bytes(), 0o644)
    return files


def _data_entries(files: dict):
    dirs = {"."}
    for path in files:
        parts = path.split("/")[:-1]
        dirs.update("./" + "/".join(parts[:i + 1]) for i in range(len(parts)))
    entries = [("./" if d == "." else d + "/", None, 0o755) for d in dirs]
    entries += [("./" + path, data, mode) for path, (data, mode) in files.items()]
    return entries


def _control(version: str, arch: str, installed_size: int) -> str:
    return (f"Package: {NAME}\nVersion: {version}\nArchitecture: {arch}\nMaintainer: {MAINTAINER}\n"
            f"Installed-Size: {installed_size}\nSection: net\nPriority: optional\nHomepage: {URL}\n"
            f"Description: {DESCRIPTION}\n")


def _control_entries(files: dict, version: str, arch: str):
    size = -(-sum(len(d) for d, _ in files.values()) // 1024)
    md5 = "".join(f"{hashlib.md5(d).hexdigest()}  {p}\n" for p, (d, _) in sorted(files.items()))
    return [("./", None, 0o755), ("./control", _control(version, arch, size).encode(), 0o644),
            ("./md5sums", md5.encode(), 0o644)]


def build_deb(binary, version, arch, out_dir, license_file, readme, sde, keyring=None, base_url=None) -> Path:
    """Write out_dir/townjump_<version>_<arch>.deb and return its path; ValueError on bad input."""
    if arch not in ARCHES:
        raise ValueError(f"unsupported architecture: {arch}")
    if not VERSION_RE.fullmatch(version):
        raise ValueError(f"version must be X.Y.Z-N: {version}")
    if keyring is not None and not base_url:
        raise ValueError("--keyring needs --base-url")
    if keyring is None and base_url:
        raise ValueError("--base-url needs --keyring")
    for label, path in (("binary", binary), ("license", license_file), ("readme", readme), ("keyring", keyring)):
        if path is not None and not Path(path).is_file():
            raise ValueError(f"{label} not found: {path}")
    files = _data_files(binary, license_file, readme, arch, keyring, base_url)
    members = [("debian-binary", b"2.0\n"),
               ("control.tar.gz", _tar_gz(_control_entries(files, version, arch), sde)),
               ("data.tar.gz", _tar_gz(_data_entries(files), sde))]
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    deb = out_dir / f"{NAME}_{version}_{arch}.deb"
    deb.write_bytes(AR_MAGIC + b"".join(_ar_member(n, d, sde) for n, d in members))
    return deb


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description="Build the townjump .deb")
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--arch", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--license", required=True, type=Path)
    p.add_argument("--readme", required=True, type=Path)
    p.add_argument("--keyring", type=Path, default=None)
    p.add_argument("--base-url", default=None)
    try:
        a = p.parse_args(argv)
    except SystemExit:
        return 2
    sde = os.environ.get("SOURCE_DATE_EPOCH", "")
    if not sde.isdigit():
        print("deb.py: SOURCE_DATE_EPOCH is required (integer)", file=sys.stderr)
        return 2
    try:
        deb = build_deb(a.binary, a.version, a.arch, a.out, a.license, a.readme, int(sde), a.keyring, a.base_url)
    except ValueError as e:
        print(f"deb.py: {e}", file=sys.stderr)
        return 2
    print(deb)
    return 0


if __name__ == "__main__":
    sys.exit(main())
