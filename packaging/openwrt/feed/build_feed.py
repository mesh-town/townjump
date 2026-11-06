#!/usr/bin/env python3
"""Builds the OpenWrt package feed: .ipk (opkg) and .apk (apk) per architecture label, signed indexes, install.sh.

build_feed.py --fetched DIR --out DIR --version X.Y.Z [--pkg-release N] [--strict] [--only ipk|apk]
              [--ephemeral-keys] [--base-url URL]
Environment: SOURCE_DATE_EPOCH (required); OPENWRT_USIGN_SEC, OPENWRT_APK_SEC (secret key files) and
OPENWRT_PUBKEYS (directory with townjump-usign.pub and townjump-apk.pem), or --ephemeral-keys; USIGN, APK
(tool paths, else PATH).
Exit codes: 0 feed built and verified, 2 bad invocation or missing input, 1 format/tool/key failure.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "common"))

import apk  # noqa: E402
import assets  # noqa: E402
import ipk  # noqa: E402
import keys  # noqa: E402
import targets  # noqa: E402

DEST = "usr/bin/townjump"


class UsageError(Exception):
    pass


def site_base_url() -> str:
    """Feed base address: OPENWRT_PAGES_URL from common/site.env (the root of the OpenWrt Pages site)."""
    for line in (HERE.parent / "common" / "site.env").read_text().splitlines():
        if line.startswith("OPENWRT_PAGES_URL="):
            return line.partition("=")[2].strip().rstrip("/")
    raise UsageError("OPENWRT_PAGES_URL is not set in common/site.env")


def render_install(base_url) -> str:
    return (HERE / "install.sh.in").read_text().replace("@BASE@", base_url.rstrip("/"))


README_TEXT = """# townjump OpenWrt feed

Package feed (opkg .ipk and apk) of townjump: a single-binary Rust port of Holesail that shares a local port over a
peer-to-peer tunnel. This repository is generated; do not edit it by hand.

Install on the router:

    wget -O- {base}/install.sh | sh

Releases, sources and documentation: https://github.com/mesh-town/townjump
"""


def parse_args(argv):
    p = argparse.ArgumentParser(prog="build_feed.py", description=__doc__.splitlines()[0])
    p.add_argument("--fetched", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--pkg-release", default="1")
    p.add_argument("--strict", action="store_true")
    p.add_argument("--only", choices=["ipk", "apk"])
    p.add_argument("--ephemeral-keys", action="store_true")
    p.add_argument("--base-url")
    return p.parse_args(argv)


def check_release(args) -> str:
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        raise UsageError(f"version must be X.Y.Z, got {args.version!r}")
    if not re.fullmatch(r"[0-9]+", args.pkg_release) or int(args.pkg_release) < 1:
        raise UsageError(f"--pkg-release must be an integer > 0, got {args.pkg_release!r}")
    return f"{args.version}-r{int(args.pkg_release)}"


def source_date_epoch() -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH", "")
    if not raw.isdigit():
        raise UsageError("SOURCE_DATE_EPOCH is required (a number of seconds)")
    return int(raw)


def check_out(args) -> Path:
    out = Path(args.out)
    if out.exists() and (not out.is_dir() or any(out.iterdir())):
        raise UsageError(f"--out {out} exists and is not empty")
    return out


def key_inputs(args, need_apk):
    """(usign secret, apk secret, pubkey dir) from the environment, or None for --ephemeral-keys."""
    if args.ephemeral_keys:
        return None
    sec, apk_sec, pub = (os.environ.get(n) for n in ("OPENWRT_USIGN_SEC", "OPENWRT_APK_SEC", "OPENWRT_PUBKEYS"))
    if not (sec and pub and (apk_sec or not need_apk)):
        raise UsageError("set OPENWRT_USIGN_SEC, OPENWRT_APK_SEC and OPENWRT_PUBKEYS, or pass --ephemeral-keys")
    return Path(sec), Path(apk_sec) if apk_sec else None, Path(pub)


def choose_binaries(args, tmp) -> dict:
    """label -> unpacked binary; a label with no archive is skipped with a notice, or refused under --strict."""
    if not Path(args.fetched).is_dir():
        raise UsageError(f"--fetched {args.fetched} is not a directory")
    archives = assets.verify_sums(args.fetched)
    have = {t for triples in targets.labels().values() for t in triples
            if f"townjump-{args.version}-{t}.tar.gz" in archives}
    picked = {label: targets.pick(label, have) for label in targets.labels()}
    missing = [label for label, triple in picked.items() if triple is None]
    if missing and args.strict:
        raise UsageError(f"no archive for labels under --strict: {', '.join(missing)}")
    for label in missing:
        print(f"notice: no archive for {label}, skipped", file=sys.stderr)
    if len(missing) == len(picked):
        raise UsageError("no label has an archive: nothing to build")
    unpacked = {t: assets.unpack(archives[f"townjump-{args.version}-{t}.tar.gz"], args.version, t, tmp / "bins")
                for t in sorted(set(picked.values()) - {None})}
    return {label: unpacked[t] for label, t in picked.items() if t is not None}


def find_usign() -> str:
    found = os.environ.get("USIGN") or shutil.which("usign")
    if not found:
        raise UsageError("usign not found: set USIGN=<path> or put usign on PATH (scripts tools.sh builds it)")
    return found


def resolve_keys(configured, usign_bin, tmp, need_apk):
    """Keys to sign with; env keys must match their public halves, one-off keys cannot disagree."""
    if configured is None:
        return keys.ephemeral_keys(tmp, usign_bin)
    usign_sec, apk_sec, pubdir = configured
    if not keys.usign_key_matches(usign_sec, pubdir / keys.USIGN_PUB, usign_bin):
        raise RuntimeError("usign secret key does not match the public key in OPENWRT_PUBKEYS")
    if need_apk and not keys.apk_key_matches(apk_sec, pubdir / keys.APK_PEM):
        raise RuntimeError("apk secret key does not match the public key in OPENWRT_PUBKEYS")
    return usign_sec, apk_sec, pubdir


def build_label(label, binary, ctx) -> list:
    made = []
    if ctx["only"] != "apk":
        d = ctx["out"] / "opkg" / label
        path = ipk.build_ipk(binary, DEST, label, ctx["ver"], d, ctx["sde"])
        ipk.write_index(d, ctx["usign_bin"], ctx["usign_sec"])
        made.append(("ipk", label, path))
    if ctx["only"] != "ipk":
        d = ctx["out"] / "apk" / label
        path = apk.build_apk(binary, DEST, label, ctx["ver"], d, ctx["sde"], ctx["apk_sec"], ctx["apk_bin"])
        apk.write_index(d, ctx["apk_sec"], ctx["pubdir"], ctx["apk_bin"])
        made.append(("apk", label, path))
    return made


def verify_feed(ctx, labels) -> None:
    if ctx["only"] == "apk":
        return
    for label in labels:
        index = ctx["out"] / "opkg" / label / "Packages"
        if not keys.verify_usign(index, ctx["pubdir"] / keys.USIGN_PUB, ctx["usign_bin"]):
            raise RuntimeError(f"opkg index of {label} does not verify")


def write_root(ctx, base_url) -> None:
    out = ctx["out"]
    (out / "install.sh").write_text(render_install(base_url))
    (out / "install.sh").chmod(0o755)
    (out / "README.md").write_text(README_TEXT.format(base=base_url.rstrip("/")))
    for name in (keys.USIGN_PUB, keys.APK_PEM):
        if (ctx["pubdir"] / name).is_file():
            shutil.copy(ctx["pubdir"] / name, out / name)


def stamp(out: Path, sde: int) -> None:
    for p in [out, *out.rglob("*")]:
        os.utime(p, (sde, sde))


def run(args, tmp) -> int:
    sde = source_date_epoch()
    ver = check_release(args)
    out = check_out(args)
    need_apk = args.only != "ipk"
    configured = key_inputs(args, need_apk)
    binaries = choose_binaries(args, tmp)
    usign_bin = find_usign()
    apk_bin = apk.find_apk() if need_apk else None
    if need_apk:
        apk.require_root(os.geteuid())
    usign_sec, apk_sec, pubdir = resolve_keys(configured, usign_bin, tmp, need_apk)
    out.mkdir(parents=True, exist_ok=True)
    ctx = {"out": out, "only": args.only, "ver": ver, "sde": sde, "usign_bin": usign_bin, "apk_bin": apk_bin,
           "usign_sec": usign_sec, "apk_sec": apk_sec, "pubdir": Path(pubdir)}
    made = [m for label, binary in binaries.items() for m in build_label(label, binary, ctx)]
    verify_feed(ctx, list(binaries))
    write_root(ctx, args.base_url or site_base_url())
    stamp(out, sde)
    for fmt, label, path in made:
        print(f"{fmt} {label} {path} {path.stat().st_size}")
    return 0


def main(argv=None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    with tempfile.TemporaryDirectory() as tmp:
        try:
            return run(args, Path(tmp))
        except (UsageError, apk.ApkMissing) as e:
            print(f"build_feed: {e}", file=sys.stderr)
            return 2
        except (assets.AssetError, ValueError, RuntimeError, subprocess.CalledProcessError, OSError) as e:
            print(f"build_feed: {e}", file=sys.stderr)
            return 1


if __name__ == "__main__":
    sys.exit(main())
