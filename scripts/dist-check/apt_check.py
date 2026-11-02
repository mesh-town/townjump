#!/usr/bin/env python3
"""Check a townjump apt repository with its own parser.

apt_check.py REPO [--pubkey KEYRING]
Checks every Packages file against the pool (Filename, Size, SHA256), Packages.gz against Packages, the
SHA256 and SHA512 lists of Release against the index files, the by-hash files, and (with --pubkey) InRelease
and Release.gpg with gpgv. Prints one line per check. Standalone on purpose: the builder is not imported.
Exit codes: 0 ok, 1 mismatch, 2 bad arguments, missing repository or missing gpgv.
"""
import argparse
import gzip
import hashlib
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ARCHES = ("amd64", "arm64", "armhf")


class Mismatch(Exception):
    pass


def need(cond, msg):
    if not cond:
        raise Mismatch(msg)


def read(path: Path) -> bytes:
    need(path.is_file(), f"missing file: {path}")
    return path.read_bytes()


def paragraphs(text: str):
    return [dict(re.findall(r"^([\w-]+): (.*)$", p, re.M)) for p in text.strip("\n").split("\n\n") if p.strip()]


def check_packages(repo: Path, stable: Path, arch: str) -> int:
    base = stable / "main" / f"binary-{arch}"
    data = read(base / "Packages")
    need(gzip.decompress(read(base / "Packages.gz")) == data, f"Packages.gz differs from Packages ({arch})")
    entries = paragraphs(data.decode())
    for e in entries:
        need(e.get("Architecture") == arch, f"{e.get('Package')} has architecture {e.get('Architecture')} in {arch}")
        pkg = read(repo / e.get("Filename", "missing-filename"))
        need(len(pkg) == int(e.get("Size", -1)), f"size of {e['Filename']} differs")
        need(hashlib.sha256(pkg).hexdigest() == e.get("SHA256"), f"SHA256 of {e['Filename']} differs")
    return len(entries)


def release_lists(text: str) -> dict:
    out, current = {}, None
    for line in text.splitlines():
        if line in ("SHA256:", "SHA512:"):
            current = out.setdefault(line[:-1], {})
        elif line.startswith(" ") and current is not None:
            digest, size, rel = line.split()
            current[rel] = (digest, int(size))
        elif not line.startswith(" "):
            current = None
    return out


def check_release(stable: Path) -> int:
    text = read(stable / "Release").decode()
    lists = release_lists(text)
    need(set(lists) == {"SHA256", "SHA512"}, f"Release must list SHA256 and SHA512, found {sorted(lists)}")
    need("MD5Sum:" not in text, "Release lists MD5Sum")
    wanted = {f"main/binary-{a}/{n}" for a in ARCHES for n in ("Packages", "Packages.gz")}
    for label, algo in (("SHA256", hashlib.sha256), ("SHA512", hashlib.sha512)):
        need(set(lists[label]) == wanted, f"{label} list does not cover exactly the indexes")
        for rel, (digest, size) in lists[label].items():
            data = read(stable / rel)
            need((algo(data).hexdigest(), len(data)) == (digest, size), f"{label} of {rel} differs")
    return len(wanted)


def check_by_hash(stable: Path) -> int:
    count = 0
    for path in stable.rglob("by-hash/SHA256/*"):
        need(hashlib.sha256(path.read_bytes()).hexdigest() == path.name, f"by-hash name is not the hash: {path}")
        count += 1
    for rel, (digest, _) in release_lists(read(stable / "Release").decode())["SHA256"].items():
        need((stable / Path(rel).parent / "by-hash" / "SHA256" / digest).is_file(), f"no by-hash file for {rel}")
    return count


def check_signatures(stable: Path, pubkey: Path) -> None:
    home = Path(tempfile.mkdtemp(prefix="apt-check-"))
    try:
        for files in (["InRelease"], ["Release.gpg", "Release"]):
            for name in files:
                need((stable / name).is_file(), f"missing file: {stable / name}")
            r = subprocess.run(["gpgv", "--homedir", str(home), "--keyring", str(pubkey), *files],
                               cwd=stable, capture_output=True, text=True)
            need(r.returncode == 0, f"gpgv rejected {files[0]}: {r.stderr.strip()[-200:]}")
            print(f"ok: {files[0]} verified")
    finally:
        shutil.rmtree(home, ignore_errors=True)


def run(repo: Path, pubkey) -> None:
    stable = repo / "dists" / "stable"
    for arch in ARCHES:
        print(f"ok: Packages {arch}: {check_packages(repo, stable, arch)} entries match the pool")
    print(f"ok: Release covers {check_release(stable)} index files (SHA256, SHA512)")
    print(f"ok: {check_by_hash(stable)} by-hash files named by their hash")
    if pubkey is None:
        print("signature: not checked")
    else:
        check_signatures(stable, pubkey)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description="Check a townjump apt repository")
    p.add_argument("repo", type=Path)
    p.add_argument("--pubkey", type=Path, default=None, help="binary keyring for gpgv")
    try:
        a = p.parse_args(argv)
    except SystemExit:
        return 2
    if not (a.repo / "dists" / "stable").is_dir():
        print(f"apt_check: no repository at {a.repo}", file=sys.stderr)
        return 2
    if a.pubkey is not None and (not a.pubkey.is_file() or shutil.which("gpgv") is None):
        print("apt_check: --pubkey needs an existing keyring and gpgv", file=sys.stderr)
        return 2
    try:
        run(a.repo, a.pubkey)
    except (Mismatch, OSError, ValueError, UnicodeDecodeError, EOFError) as e:
        print(f"apt_check: FAIL: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
