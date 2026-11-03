"""Tests of apk.py: the .apk package and its signed index.

Tests on a fake apk always run. Tests with the real apk-tools need it (APK or PATH) and, for building a
package, root or fakeroot; otherwise they are skipped, unless TOWNJUMP_PKG_REQUIRE_TOOLS=1 turns a skip into
a failure.

Run: python3 -m unittest discover -s packaging/openwrt/feed -t packaging/openwrt/feed
"""
import os
import re
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "common"))

import apk  # noqa: E402
import feed_fixtures as ff  # noqa: E402
import fixtures  # noqa: E402

SDE = 1700000000
DEST = "usr/bin/townjump"


class Case(unittest.TestCase):
    def setUp(self):
        self.tmp = ff.make_tmp(self)
        self.tools = ff.fake_tools(self.tmp)
        self.binary = ff.fake_binary(self.tmp)
        patch = mock.patch.dict(os.environ, {"FAKE_LOG": str(self.tools["log"])})
        patch.start()
        self.addCleanup(patch.stop)

    def build(self, label="aarch64_generic", version="0.2.7-r1", key=None, euid=0):
        return apk.build_apk(self.binary, DEST, label, version, self.tmp / "out", SDE, key,
                             apk_bin=self.tools["apk"], euid=euid)

    def mkpkg_line(self):
        return next(line for line in ff.calls(self.tools) if " mkpkg " in f" {line} ")


class RootCheck(Case):
    def test_ordinary_user_is_refused_with_a_hint_about_fakeroot(self):
        with self.assertRaises(PermissionError) as raised:
            apk.require_root(1000)
        self.assertIn("fakeroot", str(raised.exception))

    def test_root_is_accepted(self):
        self.assertIsNone(apk.require_root(0))

    def test_build_refuses_an_ordinary_user_before_calling_apk(self):
        with self.assertRaises(PermissionError):
            self.build(euid=1000)
        self.assertEqual(ff.calls(self.tools), [])


class FindApk(Case):
    def test_env_apk_wins_over_path(self):
        self.assertEqual(apk.find_apk({"APK": str(self.tools["apk"])}), str(self.tools["apk"]))

    def test_path_is_searched_when_env_is_unset(self):
        with mock.patch.dict(os.environ, ff.tool_env(self.tools)):
            self.assertEqual(apk.find_apk(), str(self.tools["apk"]))

    def test_missing_apk_is_a_clear_error_with_exit_code_2(self):
        with mock.patch("shutil.which", return_value=None):
            with self.assertRaises(apk.ApkMissing) as raised:
                apk.find_apk({"APK": str(self.tmp / "nope")})
        self.assertEqual(raised.exception.exit_code, 2)
        self.assertIn("APK=", str(raised.exception))


class PackageArguments(Case):
    def test_apk_has_no_scripts(self):
        self.build()
        self.assertEqual(apk.APK_SCRIPTS, ())
        self.assertNotIn("--script", " ".join(ff.calls(self.tools)))

    def test_apk_label_in_name(self):
        path = self.build(label="mipsel_24kc", version="0.2.7-r3")
        self.assertEqual(path, (self.tmp / "out" / "townjump-0.2.7-r3.apk").resolve())
        self.assertTrue(path.is_file())
        self.assertIn("arch:mipsel_24kc", self.mkpkg_line())

    def test_metadata_is_the_project_one(self):
        self.build(version="0.2.7-r3")
        line = self.mkpkg_line()
        for want in ("name:townjump", "version:0.2.7-r3", "license:AGPL-3.0-only", "origin:townjump",
                     "url:https://github.com/mesh-town/townjump",
                     "maintainer:Mesh Town <340663027+mesh-town@users.noreply.github.com>",
                     "--xattrs=no"):
            with self.subTest(want=want):
                self.assertIn(want, line)

    def test_source_date_epoch_reaches_apk(self):
        self.build()
        self.assertIn(f"sde={SDE}", ff.calls(self.tools))

    def test_only_the_binary_is_staged_and_it_is_0755(self):
        self.build()
        self.assertIn(f"exec {DEST}", ff.calls(self.tools))
        self.assertEqual([c for c in ff.calls(self.tools) if c.startswith("exec ")], [f"exec {DEST}"])

    def test_stage_tree_is_removed_after_the_build(self):
        self.build()
        self.assertEqual(sorted(p.name for p in (self.tmp / "out").iterdir()), ["townjump-0.2.7-r1.apk"])

    def test_key_signs_the_package_and_no_key_means_unsigned(self):
        self.build(key=self.tmp / "k.pem")
        self.assertIn("--sign-key", self.mkpkg_line())
        (self.tools["log"]).write_text("")
        self.build()
        self.assertNotIn("--sign-key", self.mkpkg_line())

    def test_bad_destination_is_refused(self):
        for bad in ("", "/", "usr/../bin/x", "usr//bin/x"):
            with self.subTest(dest=bad):
                with self.assertRaises(ValueError):
                    apk.build_apk(self.binary, bad, "x86_64", "0.2.7-r1", self.tmp / "out", SDE,
                                  apk_bin=self.tools["apk"], euid=0)

    def test_stage_is_removed_when_apk_fails(self):
        failing = ff.write_script(self.tmp / "fail-apk", "#!/bin/sh\nexit 1\n")
        with self.assertRaises(subprocess.CalledProcessError):
            apk.build_apk(self.binary, DEST, "x86_64", "0.2.7-r1", self.tmp / "out", SDE, apk_bin=failing, euid=0)
        self.assertEqual(list((self.tmp / "out").iterdir()), [])


class IndexArguments(Case):
    def index(self, **env):
        sec, pubdir = ff.apk_key(self.tmp / "k")
        out = self.tmp / "out"
        out.mkdir()
        (out / "townjump-0.2.7-r1.apk").write_bytes(b"x")
        with mock.patch.dict(os.environ, env):
            return apk.write_index(out, sec, pubdir, apk_bin=self.tools["apk"])

    def test_signs_with_the_key_and_returns_the_path_of_the_index(self):
        result = self.index()
        self.assertEqual(result, (self.tmp / "out" / "packages.adb").resolve())
        line = next(c for c in ff.calls(self.tools) if "mkndx" in c)
        self.assertIn("--sign-key", line)
        self.assertIn("townjump-0.2.7-r1.apk", line)

    def test_an_index_that_does_not_verify_is_an_error(self):
        with self.assertRaises(ValueError):
            self.index(FAKE_VERIFY="1")

    def test_no_packages_is_an_error(self):
        sec, pubdir = ff.apk_key(self.tmp / "k")
        (self.tmp / "empty").mkdir()
        with self.assertRaises(FileNotFoundError):
            apk.write_index(self.tmp / "empty", sec, pubdir, apk_bin=self.tools["apk"])


# ---------- tests with the real apk-tools ----------

def real_apk():
    return fixtures.require_tool("apk", "APK")


def root_or_skip():
    if os.geteuid() != 0:
        if os.environ.get("TOWNJUMP_PKG_REQUIRE_TOOLS") == "1":
            raise AssertionError("needs root or fakeroot (TOWNJUMP_PKG_REQUIRE_TOOLS=1 forbids skipping)")
        raise unittest.SkipTest("needs root or fakeroot")


def adbdump(apk_bin, path):
    done = subprocess.run([apk_bin, "adbdump", str(path)], check=True, capture_output=True, text=True)
    return done.stdout


def info_of(dump):
    return dict(re.findall(r"^  ([a-z-]+): (.*)$", dump.split("\npaths:", 1)[0], re.MULTILINE))


class RealPackage(unittest.TestCase):
    def dump(self, label="aarch64_generic", version="0.2.7-r3"):
        apk_bin = real_apk()
        root_or_skip()
        tmp = ff.make_tmp(self)
        path = apk.build_apk(ff.fake_binary(tmp), DEST, label, version, tmp / "out", SDE, apk_bin=apk_bin)
        return adbdump(apk_bin, path)

    def test_name_version_and_architecture_come_from_the_call(self):
        info = info_of(self.dump())
        self.assertEqual((info["name"], info["version"], info["arch"]), ("townjump", "0.2.7-r3", "aarch64_generic"))

    def test_real_package_has_no_scripts_and_only_the_binary(self):
        dump = self.dump()
        self.assertNotIn("scripts:", dump)
        self.assertIn("townjump", dump)
        self.assertNotIn("etc", re.findall(r"^  - name: (\S+)$", dump, re.MULTILINE))

    def test_every_entry_is_owned_by_root(self):
        dump = self.dump()
        self.assertEqual(set(re.findall(r"^\s+(?:user|group): (\S+)$", dump, re.MULTILINE)), {"root"})


class RealIndex(unittest.TestCase):
    def make_index(self):
        apk_bin = real_apk()
        tmp = ff.make_tmp(self)
        sec, pubdir = ff.apk_key(tmp / "keys")
        out = tmp / "out"
        out.mkdir()
        subprocess.run([apk_bin, "mkpkg", "--info", "name:townjump", "--info", "version:0.2.7-r1",
                        "--info", "arch:x86_64", "--output", str(out / "townjump-0.2.7-r1.apk")],
                       check=True, capture_output=True)
        return apk_bin, apk.write_index(out, sec, pubdir, apk_bin=apk_bin), pubdir, tmp

    def test_accepts_the_index_signed_with_the_matching_key(self):
        apk_bin, adb, pubdir, _ = self.make_index()
        self.assertTrue(apk.verify_apk_index(adb, pubdir, apk_bin))

    def test_rejects_an_index_with_a_damaged_byte(self):
        apk_bin, adb, pubdir, tmp = self.make_index()
        for where in (0, 30, adb.stat().st_size // 2, -20, -40):
            with self.subTest(position=where):
                data = bytearray(adb.read_bytes())
                data[where] ^= 1
                bad = tmp / f"bad-{where}.adb"
                bad.write_bytes(bytes(data))
                self.assertFalse(apk.verify_apk_index(bad, pubdir, apk_bin))

    def test_rejects_an_index_checked_against_another_key(self):
        apk_bin, adb, _, tmp = self.make_index()
        _, other = ff.apk_key(tmp / "other")
        self.assertFalse(apk.verify_apk_index(adb, other, apk_bin))


if __name__ == "__main__":
    unittest.main()
