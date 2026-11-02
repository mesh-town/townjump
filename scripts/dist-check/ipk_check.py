#!/usr/bin/env python3
"""Independent check of an opkg feed of .ipk files. Does not import the builders.

ipk_check.py DIR (--usign BIN --pubkey PUB | --unsigned)
Every directory under DIR that holds a Packages file is checked: Packages.gz equals Packages, every entry
matches its file (Filename, Size, SHA256sum, control fields), every .ipk has an entry, the .ipk layout is
debian-binary / data.tar.gz / control.tar.gz with root owners and gzip headers without time or name, and the
index signature verifies (--unsigned skips the signature).
Exit codes: 0 good, 1 a check failed, 2 bad arguments.
"""
import argparse
import gzip
import hashlib
import io
import subprocess
import sys
import tarfile
from pathlib import Path

REQUIRED = ("Package", "Version", "Architecture", "Filename", "Size", "SHA256sum", "Installed-Size", "Description")


def parse_blocks(text):
    blocks = []
    for chunk in text.split("\n\n"):
        if not chunk.strip():
            continue
        fields = {}
        for line in chunk.strip("\n").split("\n"):
            key, sep, value = line.partition(": ")
            if not sep:
                raise ValueError(f"bad line in Packages: {line!r}")
            fields[key] = value
        blocks.append(fields)
    return blocks


def gzip_header_problem(raw, what):
    if raw[:2] != b"\x1f\x8b":
        return f"{what}: not gzip"
    if raw[3] & 0x08:
        return f"{what}: gzip header carries a file name"
    if raw[4:8] != b"\0\0\0\0":
        return f"{what}: gzip header carries a time"
    return None


def read_tar(raw, what, errors):
    problem = gzip_header_problem(raw, what)
    if problem:
        errors.append(problem)
        return None
    try:
        tar = tarfile.open(fileobj=io.BytesIO(gzip.decompress(raw)))
        return [(m, tar.extractfile(m).read() if m.isfile() else None) for m in tar.getmembers()]
    except (tarfile.TarError, OSError, EOFError) as e:
        errors.append(f"{what}: unreadable archive: {e}")
        return None


def check_owners(members, what, errors):
    for m, _ in members:
        if (m.uid, m.gid, m.uname, m.gname) != (0, 0, "root", "root"):
            errors.append(f"{what}: {m.name} owner is {m.uid}:{m.gid} {m.uname}:{m.gname}, want 0:0 root:root")


def check_ipk(path, entry, errors):
    name = path.name
    raw = path.read_bytes()
    outer = read_tar(raw, name, errors)
    if outer is None:
        return
    check_owners(outer, name, errors)
    names = [m.name for m, _ in outer]
    if names != ["./debian-binary", "./data.tar.gz", "./control.tar.gz"]:
        errors.append(f"{name}: members are {names}")
        return
    blobs = {m.name: data for m, data in outer}
    if blobs["./debian-binary"] != b"2.0\n":
        errors.append(f"{name}: debian-binary is not 2.0")
    ctrl = read_tar(blobs["./control.tar.gz"], f"{name}/control.tar.gz", errors)
    data = read_tar(blobs["./data.tar.gz"], f"{name}/data.tar.gz", errors)
    if ctrl is None or data is None:
        return
    check_owners(ctrl, f"{name}/control", errors)
    check_owners(data, f"{name}/data", errors)
    ctrl_files = {m.name: d for m, d in ctrl if m.isfile()}
    if sorted(ctrl_files) != ["./control"]:
        errors.append(f"{name}: control archive holds {sorted(ctrl_files)}, want only ./control")
        return
    fields = dict(line.split(": ", 1) for line in ctrl_files["./control"].decode().strip("\n").split("\n"))
    for key in ("Package", "Version", "Architecture", "Installed-Size", "Description"):
        if fields.get(key) != entry.get(key):
            errors.append(f"{name}: control {key} {fields.get(key)!r} differs from Packages {entry.get(key)!r}")
    if "Depends" in fields:
        errors.append(f"{name}: control has Depends but the binary is static")
    files = [(m, d) for m, d in data if m.isfile()]
    if len(files) != 1 or files[0][0].mode & 0o7777 != 0o755:
        errors.append(f"{name}: data must hold exactly one 0755 file, got {[(m.name, oct(m.mode)) for m, _ in files]}")
    elif str(len(files[0][1])) != fields.get("Installed-Size"):
        errors.append(f"{name}: Installed-Size {fields.get('Installed-Size')} but the file is {len(files[0][1])} bytes")
    if name != f"{fields.get('Package')}_{fields.get('Version')}_{fields.get('Architecture')}.ipk":
        errors.append(f"{name}: file name does not follow package_version_architecture")


def check_signature(directory, usign, pubkey, errors):
    sig = directory / "Packages.sig"
    if not sig.is_file():
        errors.append(f"{directory}: Packages.sig is missing")
        return
    done = subprocess.run([usign, "-V", "-m", str(directory / "Packages"), "-p", str(pubkey)],
                          capture_output=True, text=True)
    if done.returncode != 0:
        errors.append(f"{directory}: signature does not verify: {done.stderr.strip()}")


def check_dir(directory, usign, pubkey, errors):
    packages = (directory / "Packages").read_bytes()
    gz = directory / "Packages.gz"
    if not gz.is_file():
        errors.append(f"{directory}: Packages.gz is missing")
    else:
        try:
            if gzip.decompress(gz.read_bytes()) != packages:
                errors.append(f"{directory}: Packages.gz differs from Packages")
        except (OSError, EOFError) as e:
            errors.append(f"{directory}: Packages.gz unreadable: {e}")
    try:
        blocks = parse_blocks(packages.decode())
    except (ValueError, UnicodeDecodeError) as e:
        errors.append(f"{directory}: {e}")
        return
    listed = set()
    for entry in blocks:
        missing = [k for k in REQUIRED if k not in entry]
        if missing:
            errors.append(f"{directory}: entry lacks {missing}")
            continue
        path = directory / entry["Filename"]
        listed.add(entry["Filename"])
        if not path.is_file():
            errors.append(f"{directory}: {entry['Filename']} is listed but absent")
            continue
        raw = path.read_bytes()
        if int(entry["Size"]) != len(raw):
            errors.append(f"{path.name}: Size {entry['Size']} but the file is {len(raw)} bytes")
        if entry["SHA256sum"] != hashlib.sha256(raw).hexdigest():
            errors.append(f"{path.name}: SHA256sum does not match the file")
        check_ipk(path, entry, errors)
    for path in sorted(directory.glob("*.ipk")):
        if path.name not in listed:
            errors.append(f"{directory}: {path.name} is not in Packages")
    if usign:
        check_signature(directory, usign, pubkey, errors)


def check_feed(root, usign=None, pubkey=None):
    """Error strings for every Packages directory under root; empty list = good."""
    errors = []
    dirs = sorted(p.parent for p in Path(root).rglob("Packages"))
    if not dirs:
        return [f"{root}: no Packages file found"]
    for d in dirs:
        check_dir(d, usign, pubkey, errors)
    return errors


def main(argv=None):
    p = argparse.ArgumentParser(prog="ipk_check.py", description=__doc__.splitlines()[0])
    p.add_argument("dir")
    p.add_argument("--usign")
    p.add_argument("--pubkey")
    p.add_argument("--unsigned", action="store_true")
    try:
        args = p.parse_args(argv)
    except SystemExit:
        return 2
    signed = bool(args.usign or args.pubkey)
    if signed == args.unsigned or (signed and not (args.usign and args.pubkey)):
        print("ipk_check: give --usign BIN with --pubkey PUB, or --unsigned", file=sys.stderr)
        return 2
    if not Path(args.dir).is_dir():
        print(f"ipk_check: {args.dir} is not a directory", file=sys.stderr)
        return 2
    errors = check_feed(args.dir, args.usign, args.pubkey)
    for e in errors:
        print(f"ipk_check: {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
