#!/usr/bin/env python3
"""Builds the townjump opkg feed for Keenetic routers with Entware: one .ipk and index per Entware architecture.

build_feed.py --out DIR --version X.Y.Z [--pkg-release N] --base-url URL --bin TRIPLE=PATH [...]
Needs SOURCE_DATE_EPOCH in the environment.
Layout: DIR/<entware dir>/townjump_<version>-rN_<Architecture>.ipk, Packages, Packages.gz; install.sh,
index.html, README.md and .nojekyll in DIR.
Exit codes: 0 feed built, 2 bad invocation (arguments, missing SOURCE_DATE_EPOCH), 1 failure while writing.
"""
import argparse
import os
import re
import sys
from collections import namedtuple
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "openwrt" / "common"))

import assets  # noqa: E402
import ipk  # noqa: E402

Arch = namedtuple("Arch", "dir architecture machine")
# The MIPS targets are on once the MIPS probe builds both triples.
MIPS_ENABLED = True
DEST = "opt/bin/townjump"
_BASE = {"aarch64-unknown-linux-musl": Arch("aarch64-k3.10", "aarch64-3.10", "aarch64")}
_MIPS = {
    "mipsel-unknown-linux-musl": Arch("mipselsf-k3.4", "mipsel-3.4", "mipsel"),
    "mips-unknown-linux-musl": Arch("mipssf-k3.4", "mips-3.4", "mips"),
}


class UsageError(Exception):
    pass


def arches() -> dict:
    """Rust target -> Entware feed directory, opkg Architecture, ELF machine; follows MIPS_ENABLED."""
    return {**_MIPS, **_BASE} if MIPS_ENABLED else dict(_BASE)


README_TEXT = """# townjump Keenetic feed

Entware package feed (.ipk) of townjump: a single-binary Rust port of Holesail that shares a local port over a
peer-to-peer tunnel. This repository is generated; do not edit it by hand.

Install on the router (Entware):

    wget -O- {base}/install.sh | sh

Releases, sources and documentation: https://github.com/mesh-town/townjump
"""


def render_install(base_url) -> str:
    return (HERE / "install.sh.in").read_text().replace("@BASE@", base_url.rstrip("/"))


def parse_args(argv):
    p = argparse.ArgumentParser(prog="build_feed.py", description=__doc__.splitlines()[0])
    p.add_argument("--out", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--pkg-release", default="1")
    p.add_argument("--base-url", required=True)
    p.add_argument("--bin", action="append", default=[], metavar="TRIPLE=PATH")
    return p.parse_args(argv)


def check_release(args) -> str:
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        raise UsageError(f"version must be X.Y.Z, got {args.version!r}")
    if not re.fullmatch(r"[0-9]+", args.pkg_release) or int(args.pkg_release) < 1:
        raise UsageError(f"--pkg-release must be an integer > 0, got {args.pkg_release!r}")
    return f"{args.version}-r{int(args.pkg_release)}"


def check_sde() -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH", "")
    if not re.fullmatch(r"[0-9]+", raw):
        raise UsageError("SOURCE_DATE_EPOCH must be set to an integer")
    return int(raw)


def check_bin(item) -> tuple:
    known = arches()
    triple, sep, path = item.partition("=")
    if not sep or triple not in known:
        raise UsageError(f"--bin needs TRIPLE=PATH with a known triple, got {item!r}")
    try:
        machine = assets.elf_machine(path)
    except (ValueError, OSError) as e:
        raise UsageError(str(e))
    if machine != known[triple].machine:
        raise UsageError(f"{path} is a {machine} binary, not {triple}")
    return triple, path


def check_bins(args) -> dict:
    bins = dict(check_bin(item) for item in args.bin)
    if not bins:
        raise UsageError("give --bin for at least one target")
    return bins


def check_out(args) -> Path:
    out = Path(args.out)
    if out.exists() and (not out.is_dir() or any(out.iterdir())):
        raise UsageError(f"--out {out} exists and is not empty")
    return out


def write_root(out, dirs, base_url) -> None:
    (out / "install.sh").write_text(render_install(base_url))
    (out / "install.sh").chmod(0o755)
    (out / ".nojekyll").write_text("")
    base = base_url.rstrip("/")
    (out / "README.md").write_text(README_TEXT.format(base=base))
    (out / "index.html").write_text(
        "<!doctype html><meta charset=utf-8><title>townjump Keenetic feed</title>\n"
        f"<h1>townjump Keenetic feed</h1>\n<pre>wget -O- {base}/install.sh | sh</pre>\n"
        f"<p>Entware architectures: {', '.join(dirs)}</p>\n")


def stamp(out: Path, sde: int) -> None:
    for p in [out, *out.rglob("*")]:
        os.utime(p, (sde, sde))


def run(args) -> int:
    version = check_release(args)
    sde = check_sde()
    bins = check_bins(args)
    out = check_out(args)
    out.mkdir(parents=True, exist_ok=True)
    known = arches()
    made = []
    for triple, path in bins.items():
        arch = known[triple]
        made.append(ipk.build_ipk(path, DEST, arch.architecture, version, out / arch.dir, sde))
        ipk.write_index(out / arch.dir)
    write_root(out, sorted(known[t].dir for t in bins), args.base_url)
    stamp(out, sde)
    for path in made:
        print(f"ipk {path} {path.stat().st_size}")
    return 0


def main(argv=None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        return run(args)
    except UsageError as e:
        print(f"build_feed: {e}", file=sys.stderr)
        return 2
    except OSError as e:
        print(f"build_feed: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
