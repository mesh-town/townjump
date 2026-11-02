#!/usr/bin/env python3
"""Check that a binary is a static ELF matching a release triple.

elf.py FILE --expect TRIPLE
Checks class, machine, byte order, absence of PT_INTERP and, for MIPS, soft-float and ISA revision 2
from .MIPS.abiflags. Standalone on purpose: the builders do not import it, so a builder bug cannot hide here.
Exit codes: 0 ok, 1 rejected, 2 bad arguments.
"""
import argparse
import struct
import sys
from pathlib import Path

# arch prefix of the triple -> (EI_CLASS, EI_DATA, e_machine)
EXPECT = {
    "x86_64": (2, 1, 62),
    "aarch64": (2, 1, 183),
    "armv7": (1, 1, 40),
    "mipsel": (1, 1, 8),
    "mips": (1, 2, 8),
}
PT_INTERP = 3
SHT_MIPS_ABIFLAGS = 0x7000002A
FP_ABI_SOFT = 3
EM_MIPS = 8


class Rejected(Exception):
    pass


def _program_types(data, e, is64):
    if is64:
        phoff, = struct.unpack_from(e + "Q", data, 32)
        phentsize, phnum = struct.unpack_from(e + "HH", data, 54)
    else:
        phoff, = struct.unpack_from(e + "I", data, 28)
        phentsize, phnum = struct.unpack_from(e + "HH", data, 42)
    for i in range(phnum):
        at = phoff + i * phentsize
        if at + 4 > len(data):
            raise Rejected("program header table runs past the end of the file")
        yield struct.unpack_from(e + "I", data, at)[0]


def _abiflags(data, e, is64):
    """Return the raw .MIPS.abiflags bytes or None."""
    if is64:
        shoff, = struct.unpack_from(e + "Q", data, 40)
        shentsize, shnum = struct.unpack_from(e + "HH", data, 58)
    else:
        shoff, = struct.unpack_from(e + "I", data, 32)
        shentsize, shnum = struct.unpack_from(e + "HH", data, 46)
    for i in range(shnum if shoff else 0):
        at = shoff + i * shentsize
        if at + shentsize > len(data):
            raise Rejected("section header table runs past the end of the file")
        if is64:
            typ, = struct.unpack_from(e + "I", data, at + 4)
            off, size = struct.unpack_from(e + "QQ", data, at + 24)
        else:
            typ, = struct.unpack_from(e + "I", data, at + 4)
            off, size = struct.unpack_from(e + "II", data, at + 16)
        if typ == SHT_MIPS_ABIFLAGS:
            return data[off:off + size]
    return None


def _check_mips(data, e, is64):
    raw = _abiflags(data, e, is64)
    if raw is None or len(raw) < 24:
        raise Rejected("no .MIPS.abiflags section: cannot confirm soft-float and ISA revision 2")
    _, isa_level, isa_rev, _, _, _, fp_abi = struct.unpack_from(e + "HBBBBBB", raw, 0)
    if fp_abi != FP_ABI_SOFT:
        raise Rejected(f"MIPS fp_abi is {fp_abi}, expected {FP_ABI_SOFT} (soft-float)")
    if (isa_level, isa_rev) != (32, 2):
        raise Rejected(f"MIPS ISA is level {isa_level} revision {isa_rev}, expected 32 revision 2")


def check(path, triple):
    """Raise Rejected with the first violated expectation."""
    arch = triple.split("-")[0]
    cls, order, machine = EXPECT[arch]
    data = Path(path).read_bytes()
    if len(data) < 64 or data[:4] != b"\x7fELF":
        raise Rejected("not an ELF file")
    if data[4] != cls:
        raise Rejected(f"ELF class {data[4]}, expected {cls}")
    if data[5] != order:
        raise Rejected(f"byte order {'little' if data[5] == 1 else 'big'}, "
                       f"expected {'little' if order == 1 else 'big'}")
    e = "<" if order == 1 else ">"
    got, = struct.unpack_from(e + "H", data, 18)
    if got != machine:
        raise Rejected(f"e_machine {got}, expected {machine}")
    if PT_INTERP in _program_types(data, e, cls == 2):
        raise Rejected("dynamically linked (PT_INTERP present)")
    if machine == EM_MIPS:
        _check_mips(data, e, cls == 2)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("file")
    ap.add_argument("--expect", required=True)
    args = ap.parse_args(argv)
    if args.expect.split("-")[0] not in EXPECT:
        print(f"elf.py: unknown triple {args.expect}", file=sys.stderr)
        return 2
    try:
        check(args.file, args.expect)
    except (Rejected, struct.error, OSError) as err:
        print(f"elf.py: {args.file}: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
