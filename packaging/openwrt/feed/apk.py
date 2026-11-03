"""Builds the .apk package and the signed apk index with apk-tools 3.0.x (mkpkg / mkndx).

The package holds one executable and no maintainer scripts: townjump is not a service, so there is nothing
to run on install, upgrade or removal. mkpkg takes owner ids from the files on disk, so the build runs as
uid 0 (root or fakeroot). The .apk is not byte-reproducible (apk signs and stamps it), the inputs are.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "common"))

import ipk  # noqa: E402
from keys import verify_apk_index  # noqa: E402,F401  (re-exported: the apk checks live here too)

APK_SCRIPTS = ()  # nothing is passed to mkpkg as --script
EXIT_USAGE = 2


class ApkMissing(RuntimeError):
    """No apk executable: CLI wrappers exit with EXIT_USAGE on it."""

    exit_code = EXIT_USAGE


def find_apk(env=None) -> str:
    """The apk executable: $APK first, then PATH."""
    env = os.environ if env is None else env
    for cand in (env.get("APK"), shutil.which("apk")):
        if cand and os.path.isfile(cand) and os.access(cand, os.X_OK):
            return str(cand)
    raise ApkMissing("apk-tools not found: set APK=<path> or put apk on PATH (scripts tools.sh builds it)")


def require_root(euid: int) -> None:
    """mkpkg takes owner ids from the files on disk, so the build must run as uid 0."""
    if euid != 0:
        raise PermissionError("building .apk packages needs root ownership of the staged files: "
                              "run as root or under fakeroot")


def _stage(binary, dest_path: str, stage: Path, sde: int) -> None:
    """usr/bin/townjump (0755) under stage, every mtime = sde."""
    parts = dest_path.strip("/").split("/")
    if not dest_path.strip("/") or ".." in parts or "" in parts:
        raise ValueError(f"bad destination path {dest_path!r}")
    target = stage.joinpath(*parts)
    target.parent.mkdir(parents=True)
    shutil.copyfile(binary, target)
    target.chmod(0o755)
    for path in [target, *target.parents]:
        os.utime(path, (sde, sde))
        if path == stage:
            break


def _mkpkg_cmd(apk_bin, label, version, stage, out, key):
    info = {"name": ipk.NAME, "version": version, "description": ipk.DESCRIPTION, "arch": label,
            "license": ipk.LICENSE, "origin": ipk.NAME, "url": ipk.URL, "maintainer": ipk.MAINTAINER,
            "depends": ""}
    cmd = [str(apk_bin)]
    if key is not None:
        cmd += ["--sign-key", str(Path(key).resolve())]
    cmd += ["mkpkg", "--xattrs=no"]
    for name, value in info.items():
        cmd += ["--info", f"{name}:{value}"]
    return cmd + ["--files", str(stage), "--output", str(out)]


def build_apk(binary, dest_path: str, label: str, version: str, out_dir, sde: int, key=None,
              apk_bin=None, euid=None) -> Path:
    """Package one executable (dest_path is its place on the device); version is V-rN; key signs the package."""
    require_root(os.geteuid() if euid is None else euid)
    apk_bin = apk_bin or find_apk()
    out_dir = Path(out_dir).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / f"{ipk.NAME}-{version}.apk"
    stage = out_dir / f".stage-{ipk.NAME}-{label}"
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir()
    try:
        _stage(binary, dest_path, stage, sde)
        env = {**os.environ, "SOURCE_DATE_EPOCH": str(sde)}
        subprocess.run(_mkpkg_cmd(apk_bin, label, version, stage, out, key), check=True, env=env,
                       stdout=subprocess.DEVNULL)
    finally:
        shutil.rmtree(stage, ignore_errors=True)
    return out


def write_index(out_dir, key, pubkey_dir, apk_bin=None) -> Path:
    """packages.adb over every .apk in out_dir, signed with key; checked against the public key in pubkey_dir."""
    apk_bin = apk_bin or find_apk()
    out_dir = Path(out_dir).resolve()
    packages = [str(p) for p in sorted(out_dir.glob("*.apk"))]
    if not packages:
        raise FileNotFoundError(f"no .apk files in {out_dir}")
    index = out_dir / "packages.adb"
    subprocess.run([str(apk_bin), "--allow-untrusted", "--sign-key", str(Path(key).resolve()),
                    "mkndx", "--output", str(index)] + packages, check=True, stdout=subprocess.DEVNULL)
    if not verify_apk_index(index, pubkey_dir, apk_bin):
        raise ValueError(f"{index} does not verify against the key in {pubkey_dir}")
    return index
