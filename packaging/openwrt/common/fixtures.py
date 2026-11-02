"""Test helpers shared by the packaging tests: synthetic ELF files, release assets, tool gating."""
import gzip
import hashlib
import io
import os
import shutil
import struct
import tarfile
import unittest
from pathlib import Path

_E_MACHINE = {"x86_64": 62, "aarch64": 183, "arm": 40, "mips": 8}
_TRIPLE_MACHINE = {"x86_64": "x86_64", "aarch64": "aarch64", "armv7": "arm", "mipsel": "mips", "mips": "mips"}
FP_ABI_SOFT = 3
FP_ABI_DOUBLE = 1


def make_elf(machine, endian="little", bits=64, static=True, fp_abi=None, isa_rev=2):
    """Build a minimal ELF executable header with program headers.

    A dynamic file (static=False) gets a PT_INTERP header. A MIPS file gets a .MIPS.abiflags section when
    fp_abi is not None (isa_level 32, revision isa_rev).
    """
    e = "<" if endian == "little" else ">"
    is64 = bits == 64
    ehsize, phentsize, shentsize = (64, 56, 64) if is64 else (52, 32, 40)
    ptypes = [1] + ([] if static else [3])
    phoff = ehsize
    data_off = phoff + phentsize * len(ptypes)
    phdrs = b""
    for t in ptypes:
        if is64:
            phdrs += struct.pack(e + "IIQQQQQQ", t, 5, 0, 0, 0, 0, 0, 0x1000)
        else:
            phdrs += struct.pack(e + "IIIIIIII", t, 0, 0, 0, 0, 0, 5, 0x1000)
    sections, shoff, shnum, shstrndx = b"", 0, 0, 0
    body = b""
    if fp_abi is not None:
        names = b"\0.MIPS.abiflags\0.shstrtab\0"
        abi = struct.pack(e + "HBBBBBBIIII", 0, 32, isa_rev, 0, 0, 0, fp_abi, 0, 0, 0, 0)
        abi_off = data_off
        str_off = abi_off + len(abi)
        body = abi + names
        shoff = str_off + len(names)
        shoff += (-shoff) % 8
        body += b"\0" * (shoff - data_off - len(body))
        rows = [b"\0" * shentsize,
                _shdr(e, is64, 1, 0x7000002A, abi_off, len(abi)),
                _shdr(e, is64, 16, 3, str_off, len(names))]
        sections, shnum, shstrndx = b"".join(rows), 3, 2
    flags = 0x70001004 if machine == "mips" else 0
    if is64:
        head = struct.pack(e + "HHIQQQIHHHHHH", 2, _E_MACHINE[machine], 1, 0, phoff, shoff, flags,
                           ehsize, phentsize, len(ptypes), shentsize, shnum, shstrndx)
    else:
        head = struct.pack(e + "HHIIIIIHHHHHH", 2, _E_MACHINE[machine], 1, 0, phoff, shoff, flags,
                           ehsize, phentsize, len(ptypes), shentsize, shnum, shstrndx)
    ident = b"\x7fELF" + bytes([2 if is64 else 1, 1 if endian == "little" else 2, 1]) + b"\0" * 9
    return ident + head + phdrs + body + sections + b"\0" * 16


def _shdr(e, is64, name, typ, off, size):
    if is64:
        return struct.pack(e + "IIQQQQIIQQ", name, typ & 0xFFFFFFFF, 0, 0, off, size, 0, 0, 1, 0)
    return struct.pack(e + "IIIIIIIIII", name, typ & 0xFFFFFFFF, 0, 0, off, size, 0, 0, 1, 0)


def elf_for_triple(triple, static=True):
    """A valid static ELF for a release triple such as aarch64-unknown-linux-musl."""
    arch = triple.split("-")[0]
    machine = _TRIPLE_MACHINE[arch]
    bits = 32 if arch in ("armv7", "mips", "mipsel") else 64
    endian = "big" if arch == "mips" else "little"
    fp = FP_ABI_SOFT if machine == "mips" else None
    return make_elf(machine, endian, bits, static, fp_abi=fp)


def _tar_gz(path, top, members):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w") as tar:
        for name, data in members:
            info = tarfile.TarInfo(f"{top}/{name}")
            info.size, info.mode, info.mtime = len(data), 0o755, 0
            tar.addfile(info, io.BytesIO(data))
    Path(path).write_bytes(_gzip(raw.getvalue()))


def _gzip(data):
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as gz:
        gz.write(data)
    return out.getvalue()


def make_assets(dest, version, triples, corrupt=None):
    """Write release-style archives and SHA256SUMS into dest.

    corrupt is None or (triple, kind) with kind in hash, absent_from_sums, version_dir, dynamic, machine.
    """
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    bad_triple, kind = corrupt if corrupt else (None, None)
    lines = []
    for triple in triples:
        name = f"townjump-{version}-{triple}.tar.gz"
        top = f"townjump-{version}-{triple}"
        elf = elf_for_triple(triple)
        if triple == bad_triple and kind == "version_dir":
            top = f"townjump-9.9.9-{triple}"
        if triple == bad_triple and kind == "dynamic":
            elf = elf_for_triple(triple, static=False)
        if triple == bad_triple and kind == "machine":
            other = "aarch64-unknown-linux-musl" if not triple.startswith("aarch64") else "x86_64-unknown-linux-musl"
            elf = elf_for_triple(other)
        _tar_gz(dest / name, top, [("townjump", elf), ("README.md", b"readme\n")])
        digest = hashlib.sha256((dest / name).read_bytes()).hexdigest()
        if triple == bad_triple and kind == "hash":
            (dest / name).write_bytes((dest / name).read_bytes() + b"\0")
        if not (triple == bad_triple and kind == "absent_from_sums"):
            lines.append(f"{digest}  {name}\n")
    (dest / "SHA256SUMS").write_text("".join(lines))
    return dest


def require_tool(name, env=None):
    """Path of an executable (env var override first, then PATH); skip the test when missing.

    With TOWNJUMP_PKG_REQUIRE_TOOLS=1 a missing tool fails instead of skipping.
    """
    for cand in (os.environ.get(env) if env else None, shutil.which(name)):
        if cand and os.path.isfile(cand) and os.access(cand, os.X_OK):
            return str(cand)
    if os.environ.get("TOWNJUMP_PKG_REQUIRE_TOOLS") == "1":
        raise AssertionError(f"{name} is not available (TOWNJUMP_PKG_REQUIRE_TOOLS=1 forbids skipping)")
    raise unittest.SkipTest(f"{name} is not available")
