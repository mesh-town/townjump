#!/usr/bin/env python3
"""Check the structure of a townjump .deb with its own ar/tar/control/md5sums parser.

deb_check.py DEB [--arch ARCH] [--version V-N]
Checks the ar members and their order, control fields (no Depends, Installed-Size recomputed), md5sums
against the data archive, owners and modes, the file name, and that the apt source and the key come together.
Standalone on purpose: the builder is not imported, so a builder bug cannot hide here.
Exit codes: 0 ok, 1 rejected, 2 bad arguments.
"""
import argparse
import gzip
import hashlib
import io
import re
import sys
import tarfile
from pathlib import Path

ARCHES = ("amd64", "arm64", "armhf")
MEMBERS = ["debian-binary", "control.tar.gz", "data.tar.gz"]
VERSION_RE = re.compile(r"\d+\.\d+\.\d+-[1-9]\d*")
REQUIRED = ["usr/bin/townjump", "usr/share/doc/townjump/copyright", "usr/share/doc/townjump/README.md"]
SOURCE, KEY = "etc/apt/sources.list.d/townjump.sources", "usr/share/keyrings/townjump.gpg"
FIELDS = ["Package", "Version", "Architecture", "Maintainer", "Installed-Size", "Section", "Priority",
          "Homepage", "Description"]


class Rejected(Exception):
    pass


def parse_ar(raw: bytes):
    if raw[:8] != b"!<arch>\n":
        raise Rejected("not an ar archive")
    pos, out = 8, []
    while pos < len(raw):
        head = raw[pos:pos + 60]
        if len(head) < 60 or head[58:60] != b"`\n":
            raise Rejected(f"bad ar header at {pos}")
        try:
            size = int(head[48:58])
        except ValueError:
            raise Rejected(f"bad ar size at {pos}") from None
        body = raw[pos + 60:pos + 60 + size]
        if len(body) != size:
            raise Rejected("truncated ar member")
        pos += 60 + size
        if size % 2:
            if raw[pos:pos + 1] != b"\n":
                raise Rejected("odd ar member is not padded")
            pos += 1
        out.append((head[:16].decode("ascii", "replace").strip().rstrip("/"), body))
    return out


def read_tar(gz: bytes, what: str):
    try:
        with tarfile.open(fileobj=io.BytesIO(gzip.decompress(gz)), mode="r:") as t:
            return [(m, t.extractfile(m).read() if m.isfile() else None) for m in t.getmembers()]
    except (OSError, EOFError, tarfile.TarError) as e:
        raise Rejected(f"{what} is not a readable tar.gz: {e}") from None


def parse_control(text: str) -> dict:
    fields = {}
    for line in text.splitlines():
        if line.startswith(" ") and fields and line.strip():
            last = list(fields)[-1]
            fields[last] += "\n" + line
            continue
        key, sep, value = line.partition(": ")
        if not sep:
            raise Rejected(f"unexpected control line: {line!r}")
        fields[key] = value
    return fields


def check_control(entries, data) -> tuple[dict, dict]:
    names = sorted(m.name for m, _ in entries)
    if names != [".", "./control", "./md5sums"]:
        raise Rejected(f"control.tar.gz must hold ., ./control, ./md5sums only: {names}")
    bodies = {m.name: b for m, b in entries}
    fields = parse_control(bodies["./control"].decode())
    missing = [f for f in FIELDS if f not in fields]
    if missing:
        raise Rejected(f"control lacks {missing}")
    if "Depends" in fields or "Pre-Depends" in fields:
        raise Rejected("control must not have Depends")
    if fields["Package"] != "townjump":
        raise Rejected(f"Package is {fields['Package']}")
    if not VERSION_RE.fullmatch(fields["Version"]):
        raise Rejected(f"Version is not X.Y.Z-N: {fields['Version']}")
    if fields["Architecture"] not in ARCHES:
        raise Rejected(f"Architecture is {fields['Architecture']}")
    want = -(-sum(len(b) for b in data.values()) // 1024)
    if fields["Installed-Size"] != str(want):
        raise Rejected(f"Installed-Size is {fields['Installed-Size']}, files add up to {want}")
    sums = {}
    for line in bodies["./md5sums"].decode().splitlines():
        digest, sep, path = line.partition("  ")
        if not sep or path in sums:
            raise Rejected(f"bad md5sums line: {line!r}")
        sums[path] = digest
    have = {p: hashlib.md5(b).hexdigest() for p, b in data.items()}
    for path in sorted(set(sums) | set(have)):
        if path not in sums:
            raise Rejected(f"md5sums lacks {path}")
        if path not in have:
            raise Rejected(f"md5sums lists absent file {path}")
        if sums[path] != have[path]:
            raise Rejected(f"md5sums mismatch for {path}")
    return fields, bodies


def check_data(entries) -> dict:
    data = {}
    for m, body in entries:
        if m.name != "." and not m.name.startswith("./"):
            raise Rejected(f"data entry without ./ prefix: {m.name}")
        if (m.uid, m.gid, m.uname, m.gname) != (0, 0, "root", "root"):
            raise Rejected(f"{m.name} is not owned by root:root")
        if m.isdir():
            if m.mode != 0o755:
                raise Rejected(f"directory {m.name} mode {m.mode:o}")
        elif m.isfile():
            path = m.name[2:]
            want = 0o755 if path == "usr/bin/townjump" else 0o644
            if m.mode != want:
                raise Rejected(f"{path} mode {m.mode:o}, want {want:o}")
            data[path] = body
        else:
            raise Rejected(f"unexpected entry type for {m.name}")
    if [m.name for m, _ in entries] != sorted(m.name for m, _ in entries):
        raise Rejected("data entries are not sorted")
    for path in REQUIRED:
        if path not in data:
            raise Rejected(f"data lacks {path}")
    if (SOURCE in data) != (KEY in data):
        raise Rejected("apt source and key must come together")
    extra = set(data) - set(REQUIRED) - {SOURCE, KEY}
    if extra:
        raise Rejected(f"unexpected files: {sorted(extra)}")
    return data


def check_deb(path: Path, arch=None, version=None) -> dict:
    members = parse_ar(path.read_bytes())
    if [n for n, _ in members] != MEMBERS:
        raise Rejected(f"ar members are {[n for n, _ in members]}, want {MEMBERS}")
    bodies = dict(members)
    if bodies["debian-binary"] != b"2.0\n":
        raise Rejected("debian-binary is not 2.0")
    data = check_data(read_tar(bodies["data.tar.gz"], "data.tar.gz"))
    fields, _ = check_control(read_tar(bodies["control.tar.gz"], "control.tar.gz"), data)
    if path.name != f"townjump_{fields['Version']}_{fields['Architecture']}.deb":
        raise Rejected(f"file name {path.name} does not match control")
    if arch and fields["Architecture"] != arch:
        raise Rejected(f"Architecture is {fields['Architecture']}, want {arch}")
    if version and fields["Version"] != version:
        raise Rejected(f"Version is {fields['Version']}, want {version}")
    return fields


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description="Check a townjump .deb")
    p.add_argument("deb", type=Path)
    p.add_argument("--arch", choices=ARCHES)
    p.add_argument("--version")
    try:
        a = p.parse_args(argv)
    except SystemExit:
        return 2
    if not a.deb.is_file():
        print(f"deb_check.py: no such file: {a.deb}", file=sys.stderr)
        return 2
    try:
        fields = check_deb(a.deb, a.arch, a.version)
    except Rejected as e:
        print(f"deb_check.py: {a.deb.name}: {e}", file=sys.stderr)
        return 1
    print(f"ok {a.deb.name} {fields['Version']} {fields['Architecture']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
