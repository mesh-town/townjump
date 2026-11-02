#!/usr/bin/env python3
"""Verify and unpack the release archives that the package builders consume.

assets.py --fetched DIR --out DIR --version V --triples T1,T2 [--strict]
prints one TRIPLE=PATH line per unpacked binary.
Exit codes: 0 success, 1 bad format/hash/ELF, 2 bad arguments or a missing triple under --strict.
"""
import argparse
import hashlib
import re
import struct
import sys
import tarfile
from pathlib import Path

ARCHIVE_RE = re.compile(r"^townjump-.*-linux-musl.*\.tar\.gz$")
ARCH_MACHINE = {"x86_64": "x86_64", "aarch64": "aarch64", "armv7": "arm", "mipsel": "mipsel", "mips": "mips"}
_E_MACHINE = {183: "aarch64", 40: "arm", 62: "x86_64"}
_EM_MIPS = 8
_PT_INTERP = 3


class AssetError(Exception):
    """Archive, checksum or binary does not meet the release contract."""


class MissingError(Exception):
    """A requested triple has no archive and --strict is set."""


def _read_sums(fetched):
    sums = Path(fetched) / "SHA256SUMS"
    if not sums.is_file():
        raise AssetError(f"{sums}: missing")
    table = {}
    for line in sums.read_text().splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2:
            table[parts[1].lstrip("*").strip()] = parts[0].lower()
    return table


def verify_sums(fetched) -> dict:
    """Map archive name to path; every archive in the directory must be listed in SHA256SUMS and match."""
    fetched = Path(fetched)
    table = _read_sums(fetched)
    found = {}
    for path in sorted(fetched.iterdir()):
        if not ARCHIVE_RE.match(path.name):
            continue
        want = table.get(path.name)
        if want is None:
            raise AssetError(f"{path.name}: not listed in SHA256SUMS")
        if hashlib.sha256(path.read_bytes()).hexdigest() != want:
            raise AssetError(f"{path.name}: SHA-256 mismatch")
        found[path.name] = path
    return found


def _has_interp(head, is64, endian):
    if is64:
        phoff, = struct.unpack_from(endian + "Q", head, 32)
        phentsize, phnum = struct.unpack_from(endian + "HH", head, 54)
    else:
        phoff, = struct.unpack_from(endian + "I", head, 28)
        phentsize, phnum = struct.unpack_from(endian + "HH", head, 42)
    for i in range(phnum):
        at = phoff + i * phentsize
        if at + 4 <= len(head) and struct.unpack_from(endian + "I", head, at)[0] == _PT_INTERP:
            return True
    return False


def elf_machine(path) -> str:
    """Machine of a static ELF file (aarch64, arm, x86_64, mipsel, mips); ValueError otherwise."""
    head = Path(path).read_bytes()[:65536]
    if len(head) < 64 or head[:4] != b"\x7fELF":
        raise ValueError(f"{path}: not an ELF file")
    is64 = head[4] == 2
    endian = "<" if head[5] == 1 else ">"
    machine, = struct.unpack_from(endian + "H", head, 18)
    found = ("mipsel" if endian == "<" else "mips") if machine == _EM_MIPS else _E_MACHINE.get(machine)
    if found is None:
        raise ValueError(f"{path}: unsupported ELF machine {machine}")
    if _has_interp(head, is64, endian):
        raise ValueError(f"{path}: dynamically linked (PT_INTERP), a static binary is required")
    return found


def unpack(path, version, triple, out) -> Path:
    """Extract townjump-<version>-<triple>/townjump to out/<triple>/townjump and check the ELF."""
    top = f"townjump-{version}-{triple}"
    dest = Path(out) / triple / "townjump"
    try:
        with tarfile.open(path, "r:gz") as tar:
            names = tar.getnames()
            if f"{top}/townjump" not in names:
                raise AssetError(f"{Path(path).name}: no {top}/townjump (found {names[:3]})")
            member = tar.getmember(f"{top}/townjump")
            if not member.isreg():
                raise AssetError(f"{Path(path).name}: {top}/townjump is not a regular file")
            data = tar.extractfile(member).read()
    except tarfile.TarError as err:
        raise AssetError(f"{Path(path).name}: {err}") from err
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    dest.chmod(0o755)
    want = ARCH_MACHINE.get(triple.split("-")[0])
    try:
        got = elf_machine(dest)
    except ValueError as err:
        raise AssetError(str(err)) from err
    if got != want:
        raise AssetError(f"{triple}: binary is {got}, expected {want}")
    return dest


def run(fetched, out, version, triples, strict):
    """Return (exit code, TRIPLE=PATH lines, notices)."""
    archives = verify_sums(fetched)
    lines, notices, missing = [], [], []
    for triple in triples:
        name = f"townjump-{version}-{triple}.tar.gz"
        if name not in archives:
            missing.append(triple)
            continue
        lines.append(f"{triple}={unpack(archives[name], version, triple, out)}")
    notices = [f"no archive for {t}, skipped" for t in missing]
    if missing and strict:
        raise MissingError(f"missing archives under --strict: {', '.join(missing)}")
    if not lines:
        raise AssetError("no archives for the requested triples")
    return lines, notices


def _fail(msg, code):
    print(f"assets.py: {msg}", file=sys.stderr)
    return code


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--fetched", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--triples", required=True)
    ap.add_argument("--strict", action="store_true")
    args = ap.parse_args(argv)
    triples = [t for t in args.triples.split(",") if t]
    if not triples or not Path(args.fetched).is_dir():
        return _fail("need a fetched directory and at least one triple", 2)
    try:
        lines, notices = run(args.fetched, args.out, args.version, triples, args.strict)
    except MissingError as err:
        return _fail(str(err), 2)
    except AssetError as err:
        return _fail(str(err), 1)
    for note in notices:
        print(f"notice: {note}", file=sys.stderr)
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
