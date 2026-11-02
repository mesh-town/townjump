"""Tests of install.sh as rendered by build_feed.render_install, run against a fake Entware router.

Paths under /opt are redirected into a temporary root; wget and opkg are small stubs that log their calls and
imitate what the real tools do to the files.

Run: python3 -m unittest discover -s packaging/keenetic -t packaging/keenetic
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from collections import namedtuple
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "openwrt" / "common"))

import build_feed  # noqa: E402

BASE = "http://feed.test"
ARCH = "mipselsf-k3.4"
Result = namedtuple("Result", "code out err")

WGET = """\
url=""
for a in "$@"; do case "$a" in -*) ;; *) url="$a" ;; esac; done
out=""
while [ $# -gt 0 ]; do case "$1" in -O|-qO) out="$2"; shift ;; esac; shift; done
echo "wget $url" >> "$LOG"
case "$url" in https:*) [ -e "$ROOT/ssl-ready" ] || { echo "wget: not an http URL" >&2; exit 1; } ;; esac
src="$FEED_ROOT/$(printf '%s' "$url" | sed 's#^[a-z]*://[^/]*/##')"
[ -f "$src" ] || { echo "wget: 404" >&2; exit 1; }
cp "$src" "$out"
"""
OPKG = """\
echo "opkg $*" >> "$LOG"
case "$1" in
  list-installed) [ -e "$ROOT/installed" ] && echo "townjump - 0.1.7-r1" ;;
  install) case "$*" in *wget-ssl*) touch "$ROOT/ssl-ready" ;; *townjump) touch "$ROOT/installed" ;; esac ;;
esac
exit 0
"""


def write(path, text, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


class InstallShTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.root, self.feed, self.stubs = self.tmp / "root", self.tmp / "feed", self.tmp / "stubs"
        self.log, self.script = self.tmp / "log", self.tmp / "install.sh"
        self.log.write_text("")

    def setup_router(self, with_entware=True, arch_dir=ARCH, with_index=True, conf=None, base=BASE):
        if with_entware:
            write(self.root / "opt/bin/opkg", "#!/bin/sh\n" + OPKG, 0o755)
        text = conf if conf is not None else f"src/gz entware http://bin.entware.net/{arch_dir}\ndest root /\n"
        write(self.root / "opt/etc/opkg.conf", text)
        if with_index:
            write(self.feed / arch_dir / "Packages.gz", "index\n")
        write(self.stubs / "wget", "#!/bin/sh\n" + WGET, 0o755)
        script = build_feed.render_install(base).replace("/opt/", f"{self.root}/opt/")
        write(self.script, script)

    def install(self, env=None):
        environ = {**os.environ, "PATH": f"{self.stubs}:{os.environ['PATH']}", "LOG": str(self.log),
                   "ROOT": str(self.root), "FEED_ROOT": str(self.feed), **(env or {})}
        done = subprocess.run(["sh", str(self.script)], capture_output=True, text=True, env=environ,
                              stdin=subprocess.DEVNULL, timeout=60)
        return Result(done.returncode, done.stdout, done.stderr)

    def calls(self):
        return self.log.read_text().splitlines()

    def feed_conf(self):
        return (self.root / "opt/etc/opkg/townjump.conf").read_text()

    def test_no_entware_exit_1(self):
        self.setup_router(with_entware=False)
        result = self.install()
        self.assertEqual(result.code, 1)
        self.assertIn("install Entware first", result.err)
        self.assertEqual(self.calls(), [])

    def test_no_arch_exit_2(self):
        self.setup_router(conf="dest root /\n")
        result = self.install()
        self.assertEqual(result.code, 2)
        self.assertIn("no 'src/gz entware' line", result.err)
        self.assertEqual(self.calls(), [])

    def test_arch_missing_from_feed_exit_2_and_nothing_changed(self):
        self.setup_router(arch_dir="armv7sf-k3.2", with_index=False)
        result = self.install()
        self.assertEqual(result.code, 2)
        self.assertIn("no packages for armv7sf-k3.2", result.err)
        self.assertFalse((self.root / "opt/etc/opkg/townjump.conf").exists())
        self.assertFalse(any(c.startswith("opkg") for c in self.calls()))

    def test_writes_feed_conf_and_installs(self):
        self.setup_router()
        result = self.install()
        self.assertEqual(result.code, 0, result.err)
        self.assertEqual(self.feed_conf(), f"src/gz townjump {BASE}/{ARCH}\n")
        self.assertEqual([c for c in self.calls() if c.startswith("opkg")],
                         ["opkg update", "opkg list-installed townjump", "opkg install townjump"])
        self.assertIn("townjump: installed; run 'townjump --help'", result.out)

    def test_does_not_enable_or_start_a_service(self):
        self.setup_router()
        self.install()
        self.assertFalse(any("init.d" in c or "start" in c for c in self.calls()))
        self.assertFalse((self.root / "opt/etc/init.d").exists())

    def test_second_run_upgrades_and_keeps_one_feed_line(self):
        self.setup_router()
        self.install()
        result = self.install()
        self.assertEqual(result.code, 0, result.err)
        self.assertIn("opkg upgrade townjump", self.calls())
        self.assertEqual(self.feed_conf().count("src/gz"), 1)

    def test_env_overrides_feed(self):
        self.setup_router(base="http://elsewhere.test")
        result = self.install(env={"TOWNJUMP_FEED": BASE})
        self.assertEqual(result.code, 0, result.err)
        self.assertIn(f"{BASE}/{ARCH}", self.feed_conf())

    def test_trailing_slash_in_entware_address(self):
        self.setup_router(conf=f"src/gz entware http://bin.entware.net/{ARCH}/\n")
        self.assertEqual(self.install().code, 0)

    def test_https_installs_ssl_wget_first(self):
        self.setup_router(base="https://feed.test")
        result = self.install()
        self.assertEqual(result.code, 0, result.err)
        calls = self.calls()
        self.assertIn("opkg install ca-certificates wget-ssl", calls)
        self.assertIn("opkg remove wget-nossl", calls)
        self.assertLess(calls.index("opkg install ca-certificates wget-ssl"), calls.index("opkg install townjump"))

    def test_http_never_touches_wget_packages(self):
        self.setup_router()
        self.install()
        self.assertFalse(any("wget-ssl" in c for c in self.calls()))

    def test_https_unreachable_exits_2(self):
        self.setup_router(base="https://feed.test", with_index=False)
        self.assertEqual(self.install().code, 2)

    def test_script_is_valid_posix_shell(self):
        self.setup_router()
        for shell in filter(None, ("sh", shutil.which("dash"))):
            with self.subTest(shell=shell):
                done = subprocess.run([shell, "-n", str(self.script)], capture_output=True, text=True)
                self.assertEqual(done.returncode, 0, done.stderr)


if __name__ == "__main__":
    unittest.main()
