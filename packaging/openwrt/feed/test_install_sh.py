"""Tests of install.sh as rendered by build_feed.render_install.

The script runs against a fake router: the paths under /etc and /usr/bin are redirected into a temp root,
and wget, sed, apk, opkg and opkg-key are small stubs that log their calls and imitate what the real
tools do to the files.

Run: python3 -m unittest discover -s packaging/openwrt/feed -t packaging/openwrt/feed
"""
import shutil
import subprocess
import unittest
from pathlib import Path

import build_feed
import feed_fixtures as ff

BASE = "http://feed.test"
ARCH = "aarch64_generic"
INDEX = {"apk": "apk/{a}/packages.adb", "opkg": "opkg/{a}/Packages.gz"}

WGET = """\
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -O|-qO) out="$2"; shift ;;
    -*) ;;
    *) url="$1" ;;
  esac
  shift
done
echo "wget $url" >> "$LOG"
src="$FEED_ROOT/$(printf '%s' "$url" | "$REAL_SED" 's#^[a-z]*://[^/]*/##')"
[ -f "$src" ] || { echo "wget: 404" >&2; exit 1; }
cp "$src" "$out"
"""
# BusyBox-style `sed -i SCRIPT FILE`, spelled out because BSD and GNU sed disagree about the suffix argument.
SED = """\
if [ "$1" = "-i" ]; then
  "$REAL_SED" "$2" "$3" > "$3.new" && cat "$3.new" > "$3" && rm -f "$3.new"; exit $?
fi
exec "$REAL_SED" "$@"
"""
APK = """\
echo "apk $*" >> "$LOG"
case "$1" in
  info) [ -e "$ROOT/installed" ]; exit $? ;;
  add) : > "$ROOT/installed" ;;
esac
exit 0
"""
OPKG = """\
echo "opkg $*" >> "$LOG"
[ "$1" = install ] && : > "$ROOT/installed"
[ "$1" = list-installed ] && [ -e "$ROOT/installed" ] && echo "townjump - 1"
exit 0
"""
OPKG_KEY = 'echo "opkg-key $*" >> "$LOG"\n[ "$1" = add ] && cp "$2" "$ROOT/key-added.pub"\nexit 0\n'
STUBS = {"apk": {"wget": WGET, "sed": SED, "apk": APK},
         "opkg": {"wget": WGET, "sed": SED, "opkg": OPKG, "opkg-key": OPKG_KEY}}


class Router:
    def __init__(self, tmp, manager):
        self.tmp, self.manager = tmp, manager
        self.root, self.stubs, self.feed = tmp / "root", tmp / "stubs", tmp / "feed"
        self.log = tmp / "log"
        self.log.write_text("")

    def run(self, env=None, args=()):
        environ = {"PATH": f"{self.stubs}:/usr/bin:/bin", "LOG": str(self.log), "ROOT": str(self.root),
                   "FEED_ROOT": str(self.feed), "HOME": str(self.tmp), "TMPDIR": str(self.tmp),
                   "REAL_SED": shutil.which("sed"), **(env or {})}
        done = subprocess.run([shutil.which("dash") or "sh", str(self.tmp / "install.sh"), *args],
                              capture_output=True, text=True, env=environ, stdin=subprocess.DEVNULL, timeout=60)
        return done

    def read(self, rel):
        return (self.root / rel).read_text()

    def calls(self):
        return [c for c in self.log.read_text().splitlines() if c]

    def fetched(self):
        return [c.split(" ", 1)[1] for c in self.calls() if c.startswith("wget ")]


def write(path, text, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


def setup(test, manager, with_index=True, arch=ARCH, release=True, with_manager=True):
    tmp = ff.make_tmp(test)
    router = Router(tmp, manager)
    if release:
        write(router.root / "etc/openwrt_release", f"DISTRIB_ID='OpenWrt'\nDISTRIB_ARCH='{arch}'\n")
    write(router.root / "etc/apk/repositories.d/distfeeds.list", "https://downloads.example.org/packages.adb\n")
    write(router.root / "etc/opkg/distfeeds.conf", "src/gz base https://downloads.example.org/base\n")
    write(router.feed / "townjump-apk.pem", "apk-public-key\n")
    write(router.feed / "townjump-usign.pub", "usign-public-key\n")
    if with_index:
        write(router.feed / INDEX[manager].format(a=arch), "index\n")
    for name, body in STUBS[manager].items():
        if with_manager or name in ("wget", "sed"):
            write(router.stubs / name, "#!/bin/sh\n" + body, mode=0o755)
    script = build_feed.render_install(BASE).replace("/etc/", f"{router.root}/etc/")
    write(tmp / "install.sh", script.replace("/usr/bin/townjump", f"{router.root}/usr/bin/townjump"))
    return router


class ExitCodes(unittest.TestCase):
    def test_not_openwrt_exits_1_and_touches_nothing(self):
        router = setup(self, "apk", release=False)
        done = router.run()
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(router.calls(), [])

    def test_no_label_exits_1(self):
        router = setup(self, "apk")
        write(router.root / "etc/openwrt_release", "DISTRIB_ID='OpenWrt'\n")
        self.assertEqual(router.run().returncode, 1)

    def test_neither_apk_nor_opkg_exits_1(self):
        router = setup(self, "apk", with_manager=False)
        done = router.run()
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("neither apk nor opkg", done.stderr)

    def test_argument_exits_2(self):
        router = setup(self, "apk")
        self.assertEqual(router.run(args=["--now"]).returncode, 2)
        self.assertEqual(router.calls(), [])

    def test_feed_without_the_label_exits_2_and_changes_nothing(self):
        for manager in ("apk", "opkg"):
            with self.subTest(manager=manager):
                router = setup(self, manager, with_index=False)
                done = router.run()
                self.assertEqual(done.returncode, 2, done.stderr)
                self.assertIn(ARCH, done.stderr)
                self.assertEqual([c for c in router.calls() if not c.startswith("wget")], [])


class Branches(unittest.TestCase):
    def test_apk_branch_adds_key_and_repo(self):
        router = setup(self, "apk")
        done = router.run()
        self.assertEqual(done.returncode, 0, done.stderr)
        keys_dir = router.root / "etc/apk/keys"
        self.assertEqual([k.read_text() for k in keys_dir.iterdir()], ["apk-public-key\n"])
        self.assertIn(f"{BASE}/apk/{ARCH}/packages.adb", router.read("etc/apk/repositories.d/customfeeds.list"))
        self.assertIn("apk add townjump", router.calls())
        self.assertIn("townjump: installed; run 'townjump --help'", done.stdout)

    def test_opkg_branch_adds_key_and_feed(self):
        router = setup(self, "opkg")
        done = router.run()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(router.read("key-added.pub"), "usign-public-key\n")
        self.assertIn(f"src/gz townjump {BASE}/opkg/{ARCH}", router.read("etc/opkg/customfeeds.conf"))
        self.assertIn("opkg update", router.calls())
        self.assertIn("opkg install townjump", router.calls())
        self.assertFalse(any("--force-overwrite" in c for c in router.calls()))

    def test_hand_made_binary_is_replaced_with_force_overwrite_on_opkg(self):
        router = setup(self, "opkg")
        write(router.root / "usr/bin/townjump", "old\n", mode=0o755)
        self.assertEqual(router.run().returncode, 0)
        self.assertIn("opkg install --force-overwrite townjump", router.calls())

    def test_second_run_upgrades_and_keeps_one_repository_line(self):
        for manager, config, verb in (("apk", "etc/apk/repositories.d/customfeeds.list", "apk upgrade townjump"),
                                      ("opkg", "etc/opkg/customfeeds.conf", "opkg upgrade townjump")):
            with self.subTest(manager=manager):
                router = setup(self, manager)
                router.run()
                self.assertEqual(router.run().returncode, 0)
                self.assertEqual([x for x in router.read(config).splitlines() if BASE in x].__len__(), 1)
                self.assertIn(verb, router.calls())

    def test_does_not_enable_service(self):
        text = build_feed.render_install(BASE)
        for word in ("init.d", "enable", "restart", "sysctl"):
            self.assertNotIn(word, text)
        router = setup(self, "opkg")
        router.run()
        self.assertFalse(any("init" in c for c in router.calls()))

    def test_env_overrides_feed(self):
        router = setup(self, "apk")
        done = router.run(env={"TOWNJUMP_FEED": "http://mirror.test"})
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(router.fetched() and all(u.startswith("http://mirror.test/") for u in router.fetched()))
        self.assertNotIn(BASE, router.read("etc/apk/repositories.d/customfeeds.list"))


class Text(unittest.TestCase):
    def test_placeholder_is_replaced_everywhere(self):
        template = (ff.HERE / "install.sh.in").read_text()
        text = build_feed.render_install("https://x.example/f?a=1&b=2|c")
        self.assertGreaterEqual(template.count("@BASE@"), 1)
        self.assertNotIn("@BASE@", text)
        self.assertIn("https://x.example/f?a=1&b=2|c", text)

    def test_sed_is_called_the_busybox_way(self):
        # BSD sed wants a suffix argument after -i; busybox takes it as the script
        self.assertNotIn("sed -i ''", build_feed.render_install(BASE))

    def test_script_is_valid_posix_shell_and_shellcheck_clean(self):
        script = ff.make_tmp(self) / "install.sh"
        script.write_text(build_feed.render_install(BASE))
        done = subprocess.run(["sh", "-n", str(script)], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        shellcheck = shutil.which("shellcheck")
        if shellcheck:
            done = subprocess.run([shellcheck, "-s", "sh", str(script)], capture_output=True, text=True)
            self.assertEqual(done.returncode, 0, done.stdout)


if __name__ == "__main__":
    unittest.main()
