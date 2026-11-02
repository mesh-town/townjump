#!/usr/bin/env python3
"""Independent check of apk v3 (ADB) files: .apk packages and packages.adb indexes. Does not import the builders.

adb_check.py PATH [--index packages.adb] [--expect-index-signed]
PATH is an .apk file or a directory (every .apk below it is checked, and every packages.adb below it as an index).
--index names an index file to check as well. A file named *.adb is treated as an index.
Checks follow apk-tools v3.0.8 (src/adb.h, src/adb.c, src/adb_comp.c): the compression wrapper (ADB. plain, ADBd
deflate, ADBc with an algorithm byte and a level byte, zlib or zstd), the file header (magic ADB. plus a schema:
pckg for packages, indx for indexes), the block sequence (ADB first, then SIG, then DATA; no truncated block, 8-byte
padding), the ADB block header and root value, the SIG block layout, and DATA blocks in packages.
--expect-index-signed requires a SIG block in every index.
Exit codes: 0 good, 1 a format check failed, 2 bad arguments.
"""
import argparse
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

MAGIC = b"ADB."
SCHEMA_PACKAGE = b"pckg"
SCHEMA_INDEX = b"indx"
BLOCK_ADB, BLOCK_SIG, BLOCK_DATA, BLOCK_EXT = 0, 1, 2, 3
SIGN_HEADER = 18  # sign_ver, hash_alg, 16-byte key id
MAX_SIGNATURE_LEN = 2048
SHA512 = 4
ALG_DEFLATE, ALG_ZSTD = 1, 2
TYPE_OBJECT = 0xE


class FormatError(Exception):
    pass


def zstd_decompress(data):
    try:
        from compression import zstd
        return zstd.decompress(data)
    except ImportError:
        pass
    try:
        import zstandard
        return zstandard.ZstdDecompressor().decompressobj().decompress(data)
    except ImportError:
        pass
    exe = shutil.which("zstd")
    if exe is None:
        raise FormatError("zstd stream, but no zstd decompressor is available here")
    done = subprocess.run([exe, "-dc"], input=data, capture_output=True)
    if done.returncode != 0:
        raise FormatError("zstd stream does not decompress")
    return done.stdout


def inflate(data):
    d = zlib.decompressobj(-15)
    try:
        out = d.decompress(data)
    except zlib.error as e:
        raise FormatError(f"deflate stream is broken: {e}")
    if not d.eof:
        raise FormatError("deflate stream is truncated")
    return out


def unwrap(raw):
    """Plain bytes starting with the ADB. file header, after the compression wrapper."""
    if raw[:3] != b"ADB" or len(raw) < 4:
        raise FormatError("not an ADB file (no ADB magic)")
    kind = raw[3:4]
    if kind == b".":
        return raw
    if kind == b"d":
        return inflate(raw[4:])
    if kind == b"c":
        if len(raw) < 6:
            raise FormatError("compression header is truncated")
        if raw[4] == ALG_DEFLATE:
            return inflate(raw[6:])
        if raw[4] == ALG_ZSTD:
            return zstd_decompress(raw[6:])
        raise FormatError(f"unknown compression algorithm {raw[4]}")
    raise FormatError(f"unknown ADB magic variant {kind!r}")


def split_blocks(plain):
    """(schema, [(type, payload)]) with every size checked against the data that is there."""
    if len(plain) < 8 or plain[:4] != MAGIC:
        raise FormatError("bad file magic after decompression")
    schema = plain[4:8]
    blocks, pos = [], 8
    while pos < len(plain):
        if pos + 4 > len(plain):
            raise FormatError("truncated block header")
        word, = struct.unpack_from("<I", plain, pos)
        if word >> 30 == BLOCK_EXT:
            if pos + 16 > len(plain):
                raise FormatError("truncated extended block header")
            kind, size, head = word & 0x3FFFFFFF, struct.unpack_from("<Q", plain, pos + 8)[0], 16
        else:
            kind, size, head = word >> 30, word & 0x3FFFFFFF, 4
        if size < head or pos + size > len(plain):
            raise FormatError(f"truncated or invalid block of type {kind} at offset {pos}")
        blocks.append((kind, plain[pos + head:pos + size]))
        pos += (size + 7) & ~7
        if pos > len(plain) + 7:
            raise FormatError("truncated block padding")
    return schema, blocks


def check_sequence(blocks):
    allowed = {BLOCK_ADB}
    for kind, _ in blocks:
        if kind not in allowed:
            raise FormatError(f"block of type {kind} out of order (ADB, then SIG, then DATA)")
        allowed = {BLOCK_SIG, BLOCK_DATA} if kind in (BLOCK_ADB, BLOCK_SIG) else {BLOCK_DATA}
    if not blocks:
        raise FormatError("no blocks at all")


def check_adb_block(payload):
    if len(payload) < 16:
        raise FormatError("ADB block is shorter than 16 bytes")
    compat, = payload[:1]
    if compat != 0:
        raise FormatError(f"unsupported ADB compat version {compat}")
    root, = struct.unpack_from("<I", payload, 4)
    if root >> 28 != TYPE_OBJECT or (root & 0x0FFFFFFF) + 4 > len(payload):
        raise FormatError("ADB root value is not an object inside the block")


def check_sig_block(payload):
    if not SIGN_HEADER < len(payload) <= SIGN_HEADER + MAX_SIGNATURE_LEN:
        raise FormatError(f"SIG block has a bad length {len(payload)}")
    if payload[0] != 0 or payload[1] != SHA512:
        raise FormatError(f"SIG block: sign_ver {payload[0]}, hash_alg {payload[1]} (want 0 and SHA512)")


def check_file(path, want_schema, is_index, expect_signed):
    schema, blocks = split_blocks(unwrap(Path(path).read_bytes()))
    if schema != want_schema:
        raise FormatError(f"schema {schema!r}, want {want_schema!r}")
    check_sequence(blocks)
    check_adb_block(blocks[0][1])
    sigs = [b for k, b in blocks if k == BLOCK_SIG]
    for sig in sigs:
        check_sig_block(sig)
    data = [b for k, b in blocks if k == BLOCK_DATA]
    if is_index and expect_signed and not sigs:
        raise FormatError("index has no SIG block, but a signed index is expected")
    if not is_index and not data:
        raise FormatError("package has no DATA block")
    if any(len(d) < 8 for d in data):
        raise FormatError("DATA block is shorter than its 8-byte header")


def targets(root, index):
    pkgs, idxs = [], []
    p = Path(root)
    if p.is_dir():
        pkgs = sorted(p.rglob("*.apk"))
        idxs = sorted(p.rglob("packages.adb"))
    elif p.suffix == ".adb":
        idxs = [p]
    else:
        pkgs = [p]
    return pkgs, idxs + ([Path(index)] if index else [])


def run(root, index, expect_signed):
    pkgs, idxs = targets(root, index)
    errors = []
    for items, schema, is_index in ((pkgs, SCHEMA_PACKAGE, False), (idxs, SCHEMA_INDEX, True)):
        for f in items:
            try:
                check_file(f, schema, is_index, expect_signed)
            except (FormatError, OSError) as e:
                errors.append(f"{f}: {e}")
    if not pkgs and not idxs:
        errors.append(f"{root}: no .apk or packages.adb to check")
    return errors


def main(argv=None):
    p = argparse.ArgumentParser(prog="adb_check.py", description=__doc__.splitlines()[0])
    p.add_argument("path")
    p.add_argument("--index")
    p.add_argument("--expect-index-signed", action="store_true")
    try:
        args = p.parse_args(argv)
    except SystemExit:
        return 2
    if not Path(args.path).exists() or (args.index and not Path(args.index).is_file()):
        print("adb_check: path or --index does not exist", file=sys.stderr)
        return 2
    errors = run(args.path, args.index, args.expect_index_signed)
    for e in errors:
        print(f"adb_check: {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
