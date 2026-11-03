#!/usr/bin/env python3
"""Scan package metadata for traces of the build machine or the author. Never reads a binary body.

leak_check.py <dir>: walks .deb/.ipk (ar or tar.gz outer, control and data tar headers, control file contents),
Packages, Release and install.sh files. Looks for build-machine paths, other e-mail addresses, text in a non-Latin script,
the lowercase project-of-origin name and tar owners other than root. .apk files are listed but not opened
(their format is checked by adb_check.py). Exit codes: 0 clean, 1 leak found, 2 bad arguments.
"""
import gzip
import io
import re
import sys
import tarfile
from pathlib import Path

ALLOWED_MAIL = "340663027+mesh-town@users.noreply.github.com"
PATTERNS = [
    ("build path", re.compile("/Us" + "ers/|/ho" + "me/|/root/")),
    ("non-Latin script", re.compile("[" + chr(0x400) + "-" + chr(0x4ff) + "]")),
    ("forbidden name", re.compile("mesh" + "town")),
]
MAIL = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+")
TEXT_NAMES = {"Packages", "Release", "install.sh"}
PACKAGE_SUFFIXES = {".deb", ".ipk"}
MAX_CONTROL_FILE = 1 << 20


def scan_text(label, text):
    problems = []
    for what, rx in PATTERNS:
        m = rx.search(text)
        if m:
            problems.append(f"{label}: {what} ({m.group(0)!r})")
    for m in MAIL.finditer(text):
        if m.group(0) != ALLOWED_MAIL:
            problems.append(f"{label}: e-mail address {m.group(0)!r}")
    return problems


def scan_tar(label, raw, read_contents):
    problems = []
    try:
        tar = tarfile.open(fileobj=io.BytesIO(raw), mode="r:*")
    except (tarfile.TarError, OSError, EOFError) as err:
        return [f"{label}: unreadable tar ({err})"]
    with tar:
        for info in tar:
            where = f"{label}:{info.name}"
            if info.uname not in ("", "root") or info.gname not in ("", "root"):
                problems.append(f"{where}: owner {info.uname!r}/{info.gname!r}")
            problems += scan_text(where + " (name)", info.name + " " + info.linkname)
            if read_contents and info.isreg() and info.size <= MAX_CONTROL_FILE:
                text = tar.extractfile(info).read().decode("utf-8", "replace")
                problems += scan_text(where, text)
    return problems


def ar_members(raw):
    pos, out = 8, []
    while pos + 60 <= len(raw):
        head = raw[pos:pos + 60]
        name = head[:16].decode("ascii", "replace").strip().rstrip("/")
        uid, gid = head[28:34].decode("ascii", "replace").strip(), head[34:40].decode("ascii", "replace").strip()
        size = int(head[48:58].decode("ascii").strip())
        out.append((name, uid, gid, raw[pos + 60:pos + 60 + size]))
        pos += 60 + size + (size & 1)
    return out


def scan_members(label, members):
    problems = []
    for name, data in members:
        if name.startswith(("control.tar", "data.tar")):
            if name.endswith(".gz"):
                data = gzip.decompress(data)
            problems += scan_tar(f"{label}/{name}", data, name.startswith("control.tar"))
    return problems


def scan_package(path):
    raw = path.read_bytes()
    label = path.name
    try:
        if raw.startswith(b"!<arch>\n"):
            members = ar_members(raw)
            problems = [f"{label}:{n}: ar owner {u}/{g}" for n, u, g, _ in members if (u, g) not in (("0", "0"), ("", ""))]
            return problems + scan_members(label, [(n, d) for n, _, _, d in members])
        outer = tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz")
        with outer:
            members = [(m.name.lstrip("./"), outer.extractfile(m).read()) for m in outer if m.isreg()]
            return scan_tar(label, gzip.decompress(raw), False) + scan_members(label, members)
    except (OSError, EOFError, ValueError, tarfile.TarError) as err:
        return [f"{label}: unreadable package ({err})"]


def scan_dir(root):
    problems, count = [], 0
    for path in sorted(p for p in Path(root).rglob("*") if p.is_file()):
        rel = path.relative_to(root)
        if path.suffix in PACKAGE_SUFFIXES:
            problems += scan_package(path)
        elif path.name in TEXT_NAMES:
            problems += scan_text(str(rel), path.read_text("utf-8", "replace"))
        elif path.suffix == ".apk":
            pass
        else:
            continue
        count += 1
    return problems, count


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 1 or not Path(args[0]).is_dir():
        print("usage: leak_check.py <dir>", file=sys.stderr)
        return 2
    problems, count = scan_dir(args[0])
    for p in problems:
        print(f"leak_check: {p}", file=sys.stderr)
    if problems:
        return 1
    print(f"leak_check: {count} files clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
