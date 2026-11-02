"""Tests of keys.py: usign signatures and generated one-off keys.

They need the usign tool (and openssl for the apk key); without them they are skipped, unless
TOWNJUMP_PKG_REQUIRE_TOOLS=1 turns a skip into a failure. Two tests use small fake scripts instead of the
real apk and usign, so the argument handling is covered on machines without them.

Run: python3 -m unittest discover -s packaging/openwrt/feed -t packaging/openwrt/feed
"""
import stat
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "common"))

import fixtures  # noqa: E402
import keys  # noqa: E402


class Case(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)

    def script(self, name, body):
        path = self.tmp / name
        path.write_text("#!/bin/sh\n" + body)
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path


class UsignSignature(Case):
    def make_signed(self):
        usign = fixtures.require_tool("usign", "USIGN")
        fixtures.require_tool("openssl")
        sec, _, pubdir = keys.ephemeral_keys(self.tmp / "keys", usign)
        file = self.tmp / "Packages"
        file.write_text("Package: townjump\nVersion: 0.1.7-r1\nArchitecture: x86_64\n\n")
        keys.sign_usign(file, sec, usign)
        return file, pubdir / keys.USIGN_PUB, usign

    def test_signing_writes_a_sig_file_next_to_the_signed_file(self):
        # given / when
        file, _, _ = self.make_signed()
        # then
        self.assertTrue(file.with_name("Packages.sig").is_file())

    def test_matching_public_key_accepts_the_file(self):
        # given
        file, pub, usign = self.make_signed()
        # when / then
        self.assertIs(keys.verify_usign(file, pub, usign), True)

    def test_a_changed_byte_in_the_file_is_rejected(self):
        # given
        file, pub, usign = self.make_signed()
        data = bytearray(file.read_bytes())
        data[len(data) // 2] ^= 0x01
        file.write_bytes(bytes(data))
        # when / then
        self.assertIs(keys.verify_usign(file, pub, usign), False)

    def test_another_public_key_rejects_the_file(self):
        # given
        file, _, usign = self.make_signed()
        _, _, other = keys.ephemeral_keys(self.tmp / "other", usign)
        # when / then
        self.assertIs(keys.verify_usign(file, other / keys.USIGN_PUB, usign), False)

    def test_key_pair_check_accepts_a_pair_and_refuses_a_stranger(self):
        # given
        file, pub, usign = self.make_signed()
        sec, _, other = keys.ephemeral_keys(self.tmp / "other", usign)
        # when / then
        self.assertTrue(keys.usign_key_matches(sec, other / keys.USIGN_PUB, usign))
        self.assertFalse(keys.usign_key_matches(sec, pub, usign))


class ApkKey(Case):
    def test_apk_key_is_a_p256_key_matching_the_published_pem(self):
        # given
        openssl = fixtures.require_tool("openssl")
        pubdir = self.tmp / "pub"
        pubdir.mkdir()
        # when
        sec = keys.ephemeral_apk_key(self.tmp, pubdir)
        # then
        import subprocess
        text = subprocess.run([openssl, "ec", "-in", str(sec), "-noout", "-text"], check=True,
                              capture_output=True, text=True).stdout
        self.assertTrue("prime256v1" in text or "P-256" in text, "the apk key must be on the P-256 curve")
        self.assertTrue(keys.apk_key_matches(sec, pubdir / keys.APK_PEM))

    def test_a_different_key_does_not_match_the_pem(self):
        # given
        fixtures.require_tool("openssl")
        first, second = self.tmp / "a", self.tmp / "b"
        first.mkdir()
        second.mkdir()
        sec = keys.ephemeral_apk_key(first, first)
        keys.ephemeral_apk_key(second, second)
        # when / then
        self.assertFalse(keys.apk_key_matches(sec, second / keys.APK_PEM))

    def test_every_call_makes_a_new_key(self):
        # given
        fixtures.require_tool("openssl")
        first, second = self.tmp / "a", self.tmp / "b"
        first.mkdir()
        second.mkdir()
        # when
        one, two = keys.ephemeral_apk_key(first, first), keys.ephemeral_apk_key(second, second)
        # then
        self.assertNotEqual(one.read_bytes(), two.read_bytes())


class ToolArguments(Case):
    def test_apk_verify_gets_an_absolute_keys_dir_holding_the_published_pem(self):
        # given: a fake apk that succeeds only when --keys-dir is absolute and holds the pem
        apk = self.script("apk", 'case "$2" in /*) ;; *) exit 1;; esac\n[ "$3" = verify ] && [ -f "$2/townjump-apk.pem" ]\n')
        pubdir = self.tmp / "pub"
        pubdir.mkdir()
        (pubdir / keys.APK_PEM).write_text("pem\n")
        # when / then
        self.assertTrue(keys.verify_apk_index(self.tmp / "packages.adb", pubdir, apk))
        (pubdir / keys.APK_PEM).unlink()
        with self.assertRaises(FileNotFoundError):
            keys.verify_apk_index(self.tmp / "packages.adb", pubdir, apk)

    def test_usign_calls_use_the_documented_flags(self):
        # given: a fake usign that writes <file>.sig on -S and checks it on -V
        usign = self.script("usign", 'if [ "$1" = -S ]; then echo sig > "$3.sig"; else [ -f "$3.sig" ]; fi\n')
        file = self.tmp / "Packages"
        file.write_text("x\n")
        # when
        sig = keys.sign_usign(file, self.tmp / "sec", usign)
        # then
        self.assertEqual(sig, self.tmp / "Packages.sig")
        self.assertTrue(keys.verify_usign(file, self.tmp / "pub", usign))
        sig.unlink()
        self.assertFalse(keys.verify_usign(file, self.tmp / "pub", usign))


if __name__ == "__main__":
    unittest.main()
