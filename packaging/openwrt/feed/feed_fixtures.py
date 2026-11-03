"""Fixtures for the OpenWrt feed tests: fake apk and usign on PATH, one-off keys, a ready set of release assets.

Everything generic (synthetic ELF files, release archives, tool gating) is in common/fixtures.py and is
imported from there, not repeated.
"""
import os
import shutil
import stat
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "common"))

import fixtures  # noqa: E402
import keys  # noqa: E402

from fixtures import elf_for_triple, make_assets, require_tool  # noqa: E402,F401

AARCH64 = "aarch64-unknown-linux-musl"
X86_64 = "x86_64-unknown-linux-musl"
VERSION = "0.2.7"
PACKAGE_FILES = {"usr/bin/townjump": 0o755}

# A substitute for apk: records its argument lines (for mkpkg also SOURCE_DATE_EPOCH and the 0755 files
# of the stage tree) in $FAKE_LOG, writes the file named by --output,
# and answers `verify` with $FAKE_VERIFY (default 0).
FAKE_APK = r"""#!/bin/sh
printf '%s\n' "$*" >> "${FAKE_LOG:?}"
out=""; prev=""; cmd=""; files=""
for a in "$@"; do
  case "$a" in mkpkg|mkndx|verify|--version) cmd="$a" ;; esac
  [ "$prev" = "--output" ] && out="$a"
  [ "$prev" = "--files" ] && files="$a"
  prev="$a"
done
case "$cmd" in
  mkpkg)
    printf 'sde=%s\n' "$SOURCE_DATE_EPOCH" >> "$FAKE_LOG"
    (cd "$files" && find . -type f -perm 755 | sed 's|^\./|exec |') >> "$FAKE_LOG"
    printf 'fake-adb\n' > "$out" ;;
  mkndx) printf 'fake-adb\n' > "$out" ;;
  verify) exit "${FAKE_VERIFY:-0}" ;;
esac
exit 0
"""
FAKE_USIGN = "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${FAKE_LOG:?}\"\nexit 0\n"


def make_tmp(test) -> Path:
    """A fresh directory removed when the test ends."""
    path = Path(tempfile.mkdtemp(prefix="feedtest-"))
    test.addCleanup(shutil.rmtree, path, ignore_errors=True)
    return path


def write_script(path, body) -> Path:
    path = Path(path)
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IRUSR)
    return path


def fake_tools(tmp) -> dict:
    """Fake apk and usign in tmp/bin: {'apk', 'usign', 'log', 'bin'}; calls are appended to the log file."""
    bin_dir = Path(tmp) / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    log = Path(tmp) / "calls.log"
    log.write_text("")
    return {"apk": write_script(bin_dir / "apk", FAKE_APK), "usign": write_script(bin_dir / "usign", FAKE_USIGN),
            "log": log, "bin": bin_dir}


def tool_env(tools, **extra) -> dict:
    """An environment with the fake tools first on PATH and FAKE_LOG set (APK/USIGN overrides removed)."""
    env = {k: v for k, v in os.environ.items() if k not in ("APK", "USIGN")}
    env["PATH"] = f"{tools['bin']}{os.pathsep}{env.get('PATH', '')}"
    env["FAKE_LOG"] = str(tools["log"])
    env.update({k: str(v) for k, v in extra.items()})
    return env


def calls(tools) -> list:
    """Argument lines the fake tools were called with, in order."""
    return Path(tools["log"]).read_text().splitlines()


def apk_key(tmp):
    """(secret key, directory with the public townjump-apk.pem): needs openssl only."""
    fixtures.require_tool("openssl")
    pubdir = Path(tmp) / "pub"
    pubdir.mkdir(parents=True, exist_ok=True)
    return keys.ephemeral_apk_key(tmp, pubdir), pubdir


def make_keys(tmp):
    """(usign secret, apk secret, pubdir) with a real usign; skips without usign or openssl."""
    usign = fixtures.require_tool("usign", "USIGN")
    fixtures.require_tool("openssl")
    return keys.ephemeral_keys(tmp, usign)


def make_feed_assets(tmp, version=VERSION, triples=(AARCH64, X86_64)) -> Path:
    """Release-style archives and SHA256SUMS in tmp/fetched."""
    return fixtures.make_assets(Path(tmp) / "fetched", version, list(triples))


def fake_binary(tmp, triple=AARCH64) -> Path:
    """A small static ELF executable for the triple."""
    path = Path(tmp) / "townjump-bin"
    path.write_bytes(elf_for_triple(triple))
    return path
