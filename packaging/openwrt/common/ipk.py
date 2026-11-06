"""Builds .ipk packages (gzip'd tar of debian-binary, data and control archives) and the opkg Packages index.

Shared by the OpenWrt feed and the Keenetic (Entware) feed. Output is reproducible: every timestamp is the
caller's SOURCE_DATE_EPOCH, owners are root, gzip headers carry no time and no name.
"""
import gzip
import hashlib
import io
import subprocess
import tarfile
from pathlib import Path

NAME = "townjump"
DESCRIPTION = (
    "Single-binary Rust port of Holesail: share a local port over a peer-to-peer tunnel, "
    "reach it anywhere with one key. No server of your own."
)
MAINTAINER = "Mesh Town <340663027+mesh-town@users.noreply.github.com>"
LICENSE = "AGPL-3.0-only"
URL = "https://github.com/mesh-town/townjump"


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
    return _gzip(bio.getvalue())


def _gzip(data: bytes, level: int = 9) -> bytes:
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0, compresslevel=level) as g:
        g.write(data)
    return out.getvalue()


def _data_entries(dest_path: str, binary: bytes):
    """The directory chain above dest_path plus the executable itself."""
    parts = dest_path.strip("/").split("/")
    if not dest_path.strip("/") or ".." in parts or "" in parts:
        raise ValueError(f"bad destination path {dest_path!r}")
    entries = [(".", None, 0o755)]
    for i in range(1, len(parts)):
        entries.append(("./" + "/".join(parts[:i]), None, 0o755))
    entries.append(("./" + "/".join(parts), binary, 0o755))
    return entries


def _control(label: str, version: str, installed_size: int) -> str:
    return (f"Package: {NAME}\nVersion: {version}\nSection: net\nArchitecture: {label}\n"
            f"Maintainer: {MAINTAINER}\nLicense: {LICENSE}\nURL: {URL}\n"
            f"Installed-Size: {installed_size}\nDescription: {DESCRIPTION}\n")


def build_ipk(binary, dest_path: str, label: str, version: str, out_dir, sde: int) -> Path:
    """Package one executable: dest_path is its place on the device (usr/bin/townjump or opt/bin/townjump)."""
    data = Path(binary).read_bytes()
    control = [(".", None, 0o755),
               ("./control", _control(label, version, len(data)).encode(), 0o644)]
    members = [("./debian-binary", b"2.0\n"),
               ("./data.tar.gz", _tar_gz(_data_entries(dest_path, data), sde)),
               ("./control.tar.gz", _tar_gz(control, sde))]
    bio = io.BytesIO()
    with tarfile.open(fileobj=bio, mode="w", format=tarfile.GNU_FORMAT) as t:
        for name, blob in members:
            ti = tarfile.TarInfo(name)
            ti.size, ti.mtime, ti.mode = len(blob), sde, 0o644
            ti.uid = ti.gid = 0
            ti.uname = ti.gname = "root"
            t.addfile(ti, io.BytesIO(blob))
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    f = out_dir / f"{NAME}_{version}_{label}.ipk"
    f.write_bytes(_gzip(bio.getvalue()))
    return f


def _control_of(raw: bytes) -> str:
    outer = tarfile.open(fileobj=io.BytesIO(gzip.decompress(raw)))
    inner = gzip.decompress(outer.extractfile("./control.tar.gz").read())
    return tarfile.open(fileobj=io.BytesIO(inner)).extractfile("./control").read().decode()


def write_index(out_dir, usign=None, sec_key=None) -> None:
    """Packages and Packages.gz for every .ipk in out_dir; Packages.sig when both usign and sec_key are given."""
    if (usign is None) != (sec_key is None):
        raise ValueError("usign and sec_key go together")
    out_dir = Path(out_dir)
    blocks = []
    for pkg in sorted(out_dir.glob("*.ipk")):
        raw = pkg.read_bytes()
        extra = f"Filename: {pkg.name}\nSize: {len(raw)}\nSHA256sum: {hashlib.sha256(raw).hexdigest()}\n"
        ctrl = _control_of(raw)
        at = ctrl.index("Description:")
        blocks.append(ctrl[:at] + extra + ctrl[at:])
    index = "\n".join(blocks).encode()
    if (64 + len(index)) % 128 in (110, 111):  # usign bug workaround, as in the OpenWrt SDK
        index += b"\n\n"
    (out_dir / "Packages").write_bytes(index)
    (out_dir / "Packages.gz").write_bytes(_gzip(index))
    if usign is not None:
        subprocess.run([str(usign), "-S", "-m", str(out_dir / "Packages"), "-s", str(sec_key)], check=True)
