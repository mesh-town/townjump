#!/usr/bin/env python3
"""dist_tree_check.py <tree>: checks the direct-download tree built by scripts/dist/dist-tree.sh.

Expected paths exist, no file name carries a version, SHA256SUMS lists exactly the binary files of the tree and every
sum matches. Exit codes: 0 good, 1 problem (listed on stderr), 2 bad arguments.
"""
import hashlib
import re
import sys
from pathlib import Path

EXPECTED = [
    "macos/townjump-macos-aarch64.tar.gz",
    "macos/townjump-macos-x86_64.tar.gz",
    "linux/townjump-linux-x86_64.tar.gz",
    "linux/townjump-linux-aarch64.tar.gz",
    "linux/townjump-linux-armv7hf.tar.gz",
    "linux/townjump-linux-armv7sf.tar.gz",
    "linux/townjump-linux-mipsel.tar.gz",
    "linux/townjump-linux-mips.tar.gz",
    "android/townjump-termux-aarch64.tar.gz",
    "windows/townjump.exe",
    "windows/townjump-setup.exe",
]
ROOT_FILES = ["SHA256SUMS", "version.txt", "README.md", ".nojekyll"]
OS_DIRS = ("macos", "linux", "android", "windows")
VERSION_IN_NAME = re.compile(r"\d+\.\d+\.\d+")


def check(tree: Path) -> list:
    problems = []
    for rel in EXPECTED + ROOT_FILES:
        if not (tree / rel).is_file():
            problems.append(f"missing: {rel}")
    files = sorted(
        p.relative_to(tree).as_posix() for d in OS_DIRS if (tree / d).is_dir() for p in (tree / d).rglob("*") if p.is_file()
    )
    for rel in files:
        if VERSION_IN_NAME.search(rel):
            problems.append(f"version in file name: {rel}")
    for rel in files:
        if rel not in EXPECTED and not rel.startswith("linux/debian/"):
            problems.append(f"unexpected file: {rel}")
    sums = tree / "SHA256SUMS"
    if sums.is_file():
        listed = {}
        for line in sums.read_text().splitlines():
            m = re.fullmatch(r"([0-9a-f]{64})  (\S+)", line)
            if not m:
                problems.append(f"bad SHA256SUMS line: {line!r}")
                continue
            if m.group(2) in listed:
                problems.append(f"listed twice: {m.group(2)}")
            listed[m.group(2)] = m.group(1)
        for rel, digest in listed.items():
            f = tree / rel
            if not f.is_file():
                problems.append(f"SHA256SUMS names a missing file: {rel}")
            elif hashlib.sha256(f.read_bytes()).hexdigest() != digest:
                problems.append(f"checksum mismatch: {rel}")
        for rel in files:
            if not rel.startswith("linux/debian/") and rel not in listed:
                problems.append(f"not in SHA256SUMS: {rel}")
    return problems


def main(argv) -> int:
    if len(argv) != 2 or not Path(argv[1]).is_dir():
        print("usage: dist_tree_check.py <tree>", file=sys.stderr)
        return 2
    problems = check(Path(argv[1]))
    for p in problems:
        print(f"dist_tree_check: {p}", file=sys.stderr)
    if not problems:
        print("dist_tree_check: tree is good")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
