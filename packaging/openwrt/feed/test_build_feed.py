"""Tests of the build_feed.py command line: layout, tools, exit codes and reproducibility.

Most tests run anywhere with fake apk and usign on PATH. Real signing needs usign and openssl, the
apk half needs root; without them those tests are skipped (TOWNJUMP_PKG_REQUIRE_TOOLS=1 makes a skip a failure).

Run: python3 -m unittest discover -s packaging/openwrt/feed -t packaging/openwrt/feed
"""
import os
import subprocess
import sys
import unittest
from pathlib import Path

import feed_fixtures as ff
import keys

SCRIPT = ff.HERE / "build_feed.py"
EPOCH = "1700000000"
VERSION = ff.VERSION
FIVE = ["aarch64_cortex-a53", "aarch64_cortex-a72", "aarch64_cortex-a76", "aarch64_generic", "x86_64"]
SITE_BASE = "https://mesh-town.github.io/townjump/openwrt"


class Run:
    def __init__(self, code, out, err):
        self.code, self.out, self.err = code, out, err


class Feed:
    """A temp dir with release assets, fake tools, and dummy keys wired through the environment."""

    def __init__(self, test, triples=(ff.AARCH64, ff.X86_64)):
        self.tmp = ff.make_tmp(test)
        self.fetched = ff.make_feed_assets(self.tmp, triples=triples)
        self.tools = ff.fake_tools(self.tmp)
        self.out = self.tmp / "out"
        self.pub = self.tmp / "pub"
        self.pub.mkdir()
        (self.pub / keys.USIGN_PUB).write_text("usign-public\n")
        (self.pub / keys.APK_PEM).write_text("apk-public\n")
        (self.tmp / "usign.sec").write_text("usign-secret\n")
        (self.tmp / "apk.sec").write_text("apk-secret\n")

    def env(self, **extra):
        env = ff.tool_env(self.tools, SOURCE_DATE_EPOCH=EPOCH, OPENWRT_USIGN_SEC=self.tmp / "usign.sec",
                          OPENWRT_APK_SEC=self.tmp / "apk.sec", OPENWRT_PUBKEYS=self.pub)
        env.update({k: str(v) for k, v in extra.items()})
        return {k: v for k, v in env.items() if v != "None"}

    def build(self, *args, env=None, out=None):
        argv = ["--fetched", str(self.fetched), "--out", str(out or self.out), "--version", VERSION, *args]
        done = subprocess.run([sys.executable, str(SCRIPT), *argv], capture_output=True, text=True,
                              env=env if env is not None else self.env())
        return Run(done.returncode, done.stdout, done.stderr)


def names(path):
    return sorted(p.name for p in Path(path).iterdir())


class Layout(unittest.TestCase):
    def test_ipk_only_layout(self):
        # given
        feed = Feed(self)
        # when
        done = feed.build("--only", "ipk")
        # then
        self.assertEqual(done.code, 0, done.err)
        self.assertEqual(names(feed.out / "opkg"), sorted(FIVE))
        for label in FIVE:
            have = set(names(feed.out / "opkg" / label))
            self.assertLessEqual({f"townjump_{VERSION}-r1_{label}.ipk", "Packages", "Packages.gz"}, have, label)
        self.assertFalse((feed.out / "apk").exists())
        self.assertEqual(len(done.out.splitlines()), len(FIVE))
        self.assertTrue(done.out.startswith("ipk "))

    def test_feed_root_has_install_and_pubkeys(self):
        # given
        feed = Feed(self)
        # when
        feed.build("--only", "ipk")
        # then
        install = feed.out / "install.sh"
        self.assertTrue(os.access(install, os.X_OK))
        text = install.read_text()
        self.assertNotIn("@BASE@", text)
        self.assertIn(SITE_BASE, text)
        self.assertEqual((feed.out / keys.USIGN_PUB).read_text(), "usign-public\n")
        self.assertEqual((feed.out / keys.APK_PEM).read_text(), "apk-public\n")
        self.assertFalse(list(feed.out.rglob("*.sec")), "a secret key must not be published")

    def test_base_url_flag_replaces_the_site_address(self):
        # given
        feed = Feed(self)
        # when
        feed.build("--only", "ipk", "--base-url", "https://x.example/f/")
        # then
        self.assertIn("https://x.example/f", (feed.out / "install.sh").read_text())
        self.assertNotIn(SITE_BASE, (feed.out / "install.sh").read_text())

    def test_every_file_carries_source_date_epoch_as_mtime(self):
        # given
        feed = Feed(self)
        # when
        feed.build("--only", "ipk")
        # then
        self.assertEqual({int(p.stat().st_mtime) for p in [feed.out, *feed.out.rglob("*")]}, {int(EPOCH)})


class MissingLabels(unittest.TestCase):
    def test_missing_label_strict_fails(self):
        # given: no armv7 archive, so the arm labels have no binary
        feed = Feed(self)
        # when
        done = feed.build("--only", "ipk", "--strict")
        # then
        self.assertEqual(done.code, 2, done.err)
        self.assertIn("arm_cortex-a7", done.err)
        self.assertFalse(feed.out.exists(), "nothing is built before the check")

    def test_label_without_asset_skipped_lenient(self):
        # given
        feed = Feed(self)
        # when
        done = feed.build("--only", "ipk")
        # then
        self.assertEqual(done.code, 0, done.err)
        self.assertIn("notice: no archive for arm_cortex-a7, skipped", done.err)
        self.assertNotIn("arm_cortex-a7", names(feed.out / "opkg"))

    def test_no_archive_at_all_exits_2(self):
        # given
        feed = Feed(self, triples=())
        # when
        done = feed.build("--only", "ipk")
        # then
        self.assertEqual(done.code, 2, done.err)


class Refusals(unittest.TestCase):
    def test_missing_sde_exits_2(self):
        # given
        feed = Feed(self)
        env = feed.env()
        env.pop("SOURCE_DATE_EPOCH")
        # when
        done = feed.build("--only", "ipk", env=env)
        # then
        self.assertEqual(done.code, 2, done.err)
        self.assertIn("SOURCE_DATE_EPOCH", done.err)

    def test_no_keys_exit_2(self):
        # given
        feed = Feed(self)
        env = feed.env()
        for name in ("OPENWRT_USIGN_SEC", "OPENWRT_PUBKEYS"):
            env.pop(name)
        # when
        done = feed.build("--only", "ipk", env=env)
        # then
        self.assertEqual(done.code, 2, done.err)
        self.assertFalse(feed.out.exists())

    def test_apk_branch_needs_apk_exit_2(self):
        # given: PATH holds usign only
        feed = Feed(self)
        only_usign = feed.tmp / "only-usign"
        only_usign.mkdir()
        (only_usign / "usign").symlink_to(feed.tools["usign"])
        # when
        done = feed.build(env=feed.env(PATH=only_usign))
        # then
        self.assertEqual(done.code, 2, done.err)
        self.assertIn("apk-tools not found", done.err)

    def test_missing_usign_exits_2(self):
        # given
        feed = Feed(self)
        empty = feed.tmp / "empty"
        empty.mkdir()
        # when
        done = feed.build("--only", "ipk", env=feed.env(PATH=empty))
        # then
        self.assertEqual(done.code, 2, done.err)
        self.assertIn("usign not found", done.err)

    def test_bad_version_and_release_exit_2(self):
        for args in (["--pkg-release", "0"], ["--pkg-release", "x"]):
            with self.subTest(args=args):
                feed = Feed(self)
                self.assertEqual(feed.build("--only", "ipk", *args).code, 2)
        feed = Feed(self)
        argv = [sys.executable, str(SCRIPT), "--fetched", str(feed.fetched), "--out", str(feed.out),
                "--version", "1.2", "--only", "ipk"]
        self.assertEqual(subprocess.run(argv, env=feed.env(), capture_output=True).returncode, 2)

    def test_out_that_is_not_empty_exits_2(self):
        # given
        feed = Feed(self)
        feed.out.mkdir()
        (feed.out / "stale").write_text("x")
        # when / then
        self.assertEqual(feed.build("--only", "ipk").code, 2)

    def test_fetched_that_is_not_a_directory_exits_2(self):
        # given
        feed = Feed(self)
        feed.fetched = feed.tmp / "nowhere"
        # when / then
        self.assertEqual(feed.build("--only", "ipk").code, 2)

    def test_corrupt_archive_exits_1(self):
        # given
        feed = Feed(self)
        (feed.fetched / f"townjump-{VERSION}-{ff.X86_64}.tar.gz").write_bytes(b"damaged")
        # when
        done = feed.build("--only", "ipk")
        # then
        self.assertEqual(done.code, 1, done.err)

    def test_key_that_does_not_match_exits_1(self):
        # given: this usign signs, but never verifies
        feed = Feed(self)
        ff.write_script(feed.tools["usign"], '#!/bin/sh\n[ "$1" = "-V" ] && exit 1\nexit 0\n')
        # when
        done = feed.build("--only", "ipk")
        # then
        self.assertEqual(done.code, 1, done.err)
        self.assertIn("does not match", done.err)

    @unittest.skipIf(os.geteuid() == 0, "needs an ordinary user")
    def test_apk_as_ordinary_user_exits_1(self):
        # given
        feed = Feed(self)
        # when
        done = feed.build()
        # then
        self.assertEqual(done.code, 1, done.err)
        self.assertIn("root", done.err)


class Reproducibility(unittest.TestCase):
    def test_reproducible_ipk_feed(self):
        # given
        feed = Feed(self)
        # when
        first = feed.build("--only", "ipk", out=feed.tmp / "a")
        second = feed.build("--only", "ipk", out=feed.tmp / "b")
        # then
        self.assertEqual((first.code, second.code), (0, 0), first.err + second.err)
        for label in FIVE:
            for name in (f"townjump_{VERSION}-r1_{label}.ipk", "Packages", "Packages.gz"):
                a = (feed.tmp / "a" / "opkg" / label / name).read_bytes()
                self.assertEqual(a, (feed.tmp / "b" / "opkg" / label / name).read_bytes(), name)


class RealTools(unittest.TestCase):
    def test_ephemeral_keys_sign_and_verify(self):
        # given
        usign = ff.require_tool("usign", "USIGN")
        ff.require_tool("openssl")
        feed = Feed(self)
        env = ff.tool_env(feed.tools, SOURCE_DATE_EPOCH=EPOCH, USIGN=usign)
        env["PATH"] = os.environ["PATH"]
        # when
        done = feed.build("--only", "ipk", "--ephemeral-keys", env=env)
        # then
        self.assertEqual(done.code, 0, done.err)
        index = feed.out / "opkg" / "x86_64"
        self.assertTrue((index / "Packages.sig").is_file())
        self.assertTrue(keys.verify_usign(index / "Packages", feed.out / keys.USIGN_PUB, usign))

    @unittest.skipUnless(os.geteuid() == 0, "the apk half needs root")
    def test_apk_half_with_a_fake_apk_gives_the_published_layout(self):
        # given
        secret, pub = ff.apk_key(Feed(self).tmp)
        feed = Feed(self)
        pubdir = feed.tmp / "pub2"
        pubdir.mkdir()
        (pubdir / keys.USIGN_PUB).write_text("usign-public\n")
        (pubdir / keys.APK_PEM).write_bytes((pub / keys.APK_PEM).read_bytes())
        env = feed.env(OPENWRT_APK_SEC=secret, OPENWRT_PUBKEYS=pubdir)
        # when
        done = feed.build(env=env)
        # then
        self.assertEqual(done.code, 0, done.err)
        for label in FIVE:
            self.assertEqual(names(feed.out / "apk" / label), sorted([f"townjump-{VERSION}-r1.apk", "packages.adb"]))
        self.assertIn("apk x86_64 ", done.out)


if __name__ == "__main__":
    unittest.main()
