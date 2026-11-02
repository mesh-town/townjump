#!/usr/bin/env python3
"""Keeps a signed apt repository (pool, dists/stable, InRelease, Release.gpg) built from townjump .deb files.

apt_repo.py update REPO INCOMING --keep N --key SECRET.asc --pubkey KEYRING.gpg
Exit codes: 0 ok, 1 bad packages / signature / gpg failure, 2 bad arguments.
"""
import argparse
import email.utils
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from apt_support import (RepoError, add_to_pool, check_incoming, gzip_bytes, packages_index,  # noqa: E402
                         prune_pool, release_text, sha, write_direct_links)


def _release_parts(stable: Path):
    try:
        return (Path(stable) / "Release").read_text().split("SHA256:\n")
    except (OSError, UnicodeDecodeError):
        return None


def previous_hashes(stable: Path) -> dict:
    """The index files named by the published Release, found by their SHA256: {(folder, digest): bytes}."""
    parts = _release_parts(stable)
    out = {}
    block = parts[1].split("SHA512:\n")[0] if parts and len(parts) > 1 else ""
    for line in block.splitlines():
        fields = line.split()
        if len(fields) != 3:
            continue
        digest, _, rel = fields
        folder = Path(rel).parent
        data = _read_old(Path(stable), folder, rel, digest)
        if data is not None:
            out[(folder.as_posix(), digest)] = data
    return out


def _read_old(stable: Path, folder: Path, rel: str, digest: str):
    for cand in (stable / folder / "by-hash" / "SHA256" / digest, stable / rel):
        if cand.is_file() and sha(cand.read_bytes()) == digest:
            return cand.read_bytes()
    return None


def _old_date(stable: Path):
    parts = _release_parts(stable)
    m = re.search(r"^Date: (.*)$", parts[0], re.M) if parts else None
    try:
        return email.utils.parsedate_to_datetime(m.group(1)).timestamp() if m else None
    except (TypeError, ValueError):
        return None


def _put(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def write_dists(repo: Path, pool: dict, now: float) -> Path:
    """Rewrites dists/stable; the previous indexes stay reachable by hash. The Date never goes backwards."""
    stable = Path(repo) / "dists" / "stable"
    prev, old = previous_hashes(stable), _old_date(stable)
    ts = int(now) if old is None or old < int(now) else int(old) + 1
    files = {}
    for arch in sorted(pool):
        data = packages_index(pool[arch])
        files[f"main/binary-{arch}/Packages"] = data
        files[f"main/binary-{arch}/Packages.gz"] = gzip_bytes(data)
    shutil.rmtree(Path(repo) / "dists", ignore_errors=True)
    for rel, data in files.items():
        _put(stable / rel, data)
        _put(stable / Path(rel).parent / "by-hash" / "SHA256" / sha(data), data)
    for (folder, digest), data in prev.items():
        _put(stable / folder / "by-hash" / "SHA256" / digest, data)
    _put(stable / "Release", release_text(files, ts).encode())
    return stable / "Release"


def _gpg(home: Path, *args, tool="gpg", cwd=None) -> str:
    cmd = [tool, "--homedir", str(home)] + (["--batch"] if tool == "gpg" else []) + [str(a) for a in args]
    return subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=cwd).stdout


def _fingerprints(listing: str) -> list:
    out, after_sec = [], False
    for line in listing.splitlines():
        f = line.split(":")
        if f[0] == "sec":
            after_sec = True
        elif f[0] == "fpr" and after_sec:
            out.append(f[9])
            after_sec = False
        elif f[0] in ("ssb", "pub"):
            after_sec = False
    return out


def sign_release(stable: Path, key: Path) -> None:
    """InRelease (clearsigned) and Release.gpg (detached), SHA512 digest, key kept in a throwaway GNUPGHOME."""
    home = Path(tempfile.mkdtemp(prefix="tj-gpg-"))
    try:
        _gpg(home, "--import", Path(key).resolve())
        fprs = _fingerprints(_gpg(home, "--with-colons", "--list-secret-keys"))
        if len(fprs) != 1:
            raise RepoError(f"expected one secret key in {key}, found {len(fprs)}")
        sign = ["--yes", "--local-user", fprs[0], "--digest-algo", "SHA512"]
        _gpg(home, *sign, "--clearsign", "-o", stable / "InRelease", stable / "Release")
        _gpg(home, *sign, "--armor", "--detach-sign", "-o", stable / "Release.gpg", stable / "Release")
    finally:
        subprocess.run(["gpgconf", "--homedir", str(home), "--kill", "gpg-agent"], capture_output=True)
        shutil.rmtree(home, ignore_errors=True)


def verify_release(stable: Path, pubkey: Path) -> None:
    home = Path(tempfile.mkdtemp(prefix="tj-gpg-"))
    try:
        for files in (["InRelease"], ["Release.gpg", "Release"]):
            r = subprocess.run(["gpgv", "--homedir", str(home), "--keyring", str(Path(pubkey).resolve()), *files],
                               cwd=stable, capture_output=True, text=True)
            if r.returncode != 0:
                raise RepoError(f"signature check failed for {files[0]}: {r.stderr.strip()}")
    finally:
        shutil.rmtree(home, ignore_errors=True)


def _clock(now):
    if now is not None:
        return now
    sde = os.environ.get("SOURCE_DATE_EPOCH", "")
    return float(sde) if sde.isdigit() else time.time()


def update(repo, incoming, keep, key, pubkey, now=None) -> None:
    repo, incoming = Path(repo), Path(incoming)
    if keep < 1:
        raise RepoError("--keep must be at least 1")
    for label, f in (("key", key), ("public key", pubkey)):
        if not Path(f).is_file():
            raise RepoError(f"{label} file not found: {f}")
    if not incoming.is_dir():
        raise RepoError(f"incoming folder not found: {incoming}")
    debs = check_incoming(incoming)
    repo.mkdir(parents=True, exist_ok=True)
    add_to_pool(repo, debs)
    pool = prune_pool(repo, keep)
    write_direct_links(repo, pool)
    release = write_dists(repo, pool, _clock(now))
    sign_release(release.parent, Path(key))
    verify_release(release.parent, Path(pubkey))


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="apt_repo.py", description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    u = sub.add_parser("update", help="add packages from INCOMING to REPO and re-sign it")
    u.add_argument("repo", type=Path)
    u.add_argument("incoming", type=Path)
    u.add_argument("--keep", type=int, required=True, help="versions kept per architecture")
    u.add_argument("--key", type=Path, required=True, help="armored secret key")
    u.add_argument("--pubkey", type=Path, required=True, help="binary public keyring")
    return p


def main(argv=None) -> int:
    try:
        a = _parser().parse_args(argv)
    except SystemExit as e:
        return int(e.code or 0)
    try:
        update(a.repo, a.incoming, a.keep, a.key, a.pubkey)
    except (RepoError, ValueError, OSError) as e:
        print(f"apt_repo: {e}", file=sys.stderr)
        return 1
    except subprocess.CalledProcessError as e:
        tail = "\n".join((e.stderr or "").splitlines()[-20:])
        print(f"apt_repo: {e.cmd[0]} failed ({e.returncode})\n{tail}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
