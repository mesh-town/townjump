"""Tests of the scripts/dist/*-build.sh builders on synthetic release assets.

The builders run from a copy of the tree, so the presence of the committed apt keyring in the real tree
does not change the outcome. """
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT / "packaging" / "openwrt" / "common"))
sys.path.insert(0, str(ROOT / "packaging" / "openwrt" / "feed"))

import fixtures  # noqa: E402
import keys as feed_keys  # noqa: E402

VERSION = "0.1.7"
DEB_TRIPLES = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl", "armv7-unknown-linux-musleabihf"]
KEENETIC_TRIPLES = ["aarch64-unknown-linux-musl", "mipsel-unknown-linux-musl", "mips-unknown-linux-musl"]
SDE = "1700000000"


def make_key(dest):
    """A throwaway ed25519 key in its own gpg home: (armored secret key, binary public keyring)."""
    fixtures.require_tool("gpg")
    fixtures.require_tool("gpgv")
    home = Path(tempfile.mkdtemp(prefix="tjk-"))
    dest.mkdir(parents=True, exist_ok=True)
    try:
        base = ["gpg", "--homedir", str(home), "--batch", "--no-tty", "--pinentry-mode", "loopback", "--passphrase", ""]
        subprocess.run(base + ["--quick-generate-key", "Test <test@example.invalid>", "ed25519", "sign", "never"],
                       check=True, capture_output=True)
        secret, public = dest / "secret.asc", dest / "public.gpg"
        secret.write_bytes(subprocess.run(base + ["--armor", "--export-secret-keys"], check=True, capture_output=True).stdout)
        public.write_bytes(subprocess.run(base + ["--export"], check=True, capture_output=True).stdout)
        return secret, public
    finally:
        subprocess.run(["gpgconf", "--homedir", str(home), "--kill", "gpg-agent"], capture_output=True)
        shutil.rmtree(home, ignore_errors=True)


class BuildTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="tj-dist-build-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.tree = self.tmp / "tree"
        skip = shutil.ignore_patterns("__pycache__", "keys", "fixtures")
        for sub in ("scripts/dist", "scripts/dist-check", "packaging"):
            shutil.copytree(ROOT / sub, self.tree / sub, ignore=skip)
        for name in ("LICENSE", "README.md"):
            shutil.copy(ROOT / name, self.tree / name)
        self.out = self.tmp / "out"

    def assets(self, triples):
        return fixtures.make_assets(self.tmp / "fetched", VERSION, triples)

    def build(self, script, *args, env=None, sde=SDE):
        full = {k: v for k, v in os.environ.items()
                if k not in ("SOURCE_DATE_EPOCH", "TOWNJUMP_APT_KEY", "TOWNJUMP_APT_PUBKEY", "GITHUB_OUTPUT",
                             "OPENWRT_USIGN_SEC", "OPENWRT_APK_SEC", "OPENWRT_PUBKEYS", "USIGN", "APK")}
        if sde is not None:
            full["SOURCE_DATE_EPOCH"] = sde
        full.update(env or {})
        return subprocess.run([str(self.tree / "scripts" / "dist" / script), *map(str, args)],
                              capture_output=True, text=True, timeout=300, env=full)

    def deb_args(self):
        return self.tmp / "fetched", self.out, VERSION, "1"

    def commit_pubkey(self, pub):
        keys = self.tree / "packaging" / "debian" / "keys"
        keys.mkdir(parents=True)
        shutil.copy(pub, keys / "townjump.gpg")


class DebBuildTests(BuildTestCase):
    def test_deb_no_key_signed_false_no_apt(self):
        self.assets(DEB_TRIPLES)
        r = self.build("deb-build.sh", *self.deb_args())
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=false")
        names = sorted(p.name for p in (self.out / "debs").glob("*.deb"))
        self.assertEqual(names, [f"townjump_{VERSION}-1_{a}.deb" for a in ("amd64", "arm64", "armhf")])
        self.assertFalse((self.out / "apt").exists())

    def test_build_missing_sde_exits_2(self):
        self.assets(DEB_TRIPLES + KEENETIC_TRIPLES[1:])
        self.assertEqual(self.build("deb-build.sh", *self.deb_args(), sde=None).returncode, 2)
        r = self.build("keenetic-build.sh", self.tmp / "fetched", self.out, VERSION, "1", sde=None)
        self.assertEqual(r.returncode, 2)
        self.assertIn("SOURCE_DATE_EPOCH", r.stderr)

    def test_deb_key_without_committed_pubkey_exits_2(self):
        self.assets(DEB_TRIPLES)
        secret, _ = make_key(self.tmp / "keys")
        r = self.build("deb-build.sh", *self.deb_args(), env={"TOWNJUMP_APT_KEY": str(secret)})
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("no public keyring", r.stderr)
        self.assertNotIn("signed=", r.stdout)

    def test_deb_ephemeral_key_never_signed(self):
        self.assets(DEB_TRIPLES)
        secret, pub = make_key(self.tmp / "keys")
        gh = self.tmp / "gh-output"
        r = self.build("deb-build.sh", *self.deb_args(),
                       env={"TOWNJUMP_APT_KEY": str(secret), "TOWNJUMP_APT_PUBKEY": str(pub), "GITHUB_OUTPUT": str(gh)})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=false")
        self.assertEqual(gh.read_text(), "signed=false\n")
        self.assertTrue((self.out / "apt" / "dists" / "stable" / "InRelease").is_file())

    def test_deb_committed_key_signed_true(self):
        self.assets(DEB_TRIPLES)
        secret, pub = make_key(self.tmp / "keys")
        self.commit_pubkey(pub)
        r = self.build("deb-build.sh", *self.deb_args(), env={"TOWNJUMP_APT_KEY": str(secret)})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=true")

    def test_deb_missing_archive_fails(self):
        self.assets(DEB_TRIPLES[:2])
        self.assertNotEqual(self.build("deb-build.sh", *self.deb_args()).returncode, 0)

    def test_deb_update_needs_key_file(self):
        r = self.build("deb-update.sh", self.out / "apt", self.out / "debs")
        self.assertEqual(r.returncode, 2)
        self.assertEqual(self.build("deb-update.sh", "only-one").returncode, 2)


class KeeneticBuildTests(BuildTestCase):
    def keenetic(self, *extra):
        return self.build("keenetic-build.sh", self.tmp / "fetched", self.out, VERSION, "1", *extra)

    def test_keenetic_strict_missing_triple_fails(self):
        self.assets(KEENETIC_TRIPLES[:1])
        r = self.keenetic("--strict")
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertFalse((self.out / "pages").exists())

    def test_keenetic_lenient_skips_missing_triple(self):
        self.assets(KEENETIC_TRIPLES[:1])
        r = self.keenetic()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("no archive for mipsel-unknown-linux-musl", r.stderr)
        pages = self.out / "pages"
        self.assertTrue((pages / "install.sh").is_file())
        self.assertEqual(len(list(pages.rglob("*.ipk"))), 1)
        self.assertNotIn("signed=", r.stdout)

    def test_keenetic_all_triples(self):
        self.assets(KEENETIC_TRIPLES)
        r = self.keenetic("--strict")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(list((self.out / "pages").rglob("*.ipk"))), 3)

    def test_keenetic_bad_arguments_exit_2(self):
        self.assertEqual(self.build("keenetic-build.sh", "a", "b").returncode, 2)


OPENWRT_TRIPLES = ["x86_64-unknown-linux-musl"]


class OpenwrtBuildTests(BuildTestCase):
    def openwrt(self, *extra, env=None, sde=SDE):
        return self.build("openwrt-build.sh", self.tmp / "fetched", self.out, VERSION, "1", *extra, env=env, sde=sde)

    def test_openwrt_ephemeral_never_signed(self):
        fixtures.require_tool("usign")
        self.assets(OPENWRT_TRIPLES)
        gh = self.tmp / "gh-output"
        r = self.openwrt("--ephemeral-keys", "--only", "ipk", env={"GITHUB_OUTPUT": str(gh)})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=false")
        self.assertEqual(gh.read_text(), "signed=false\n")
        self.assertTrue(list((self.out / "pages" / "opkg").rglob("*.ipk")))

    def test_openwrt_committed_keys_signed_true(self):
        fixtures.require_tool("usign")
        fixtures.require_tool("apk")
        self.assets(OPENWRT_TRIPLES)
        usign_sec, apk_sec, pubdir = feed_keys.ephemeral_keys(self.tmp / "gen", shutil.which("usign"))
        committed = self.tree / "packaging" / "openwrt" / "keys"
        committed.mkdir(parents=True)
        for name in (feed_keys.USIGN_PUB, feed_keys.APK_PEM):
            shutil.copy(pubdir / name, committed / name)
        gh = self.tmp / "gh-output"
        env = {"OPENWRT_USIGN_SEC": str(usign_sec), "OPENWRT_APK_SEC": str(apk_sec), "GITHUB_OUTPUT": str(gh)}
        r = self.openwrt(env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=true")
        self.assertEqual(gh.read_text(), "signed=true\n")
        gh.unlink()
        r = self.openwrt("--only", "ipk", env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=false")
        shutil.rmtree(committed)
        shutil.rmtree(self.out)
        r = self.openwrt(env={**env, "OPENWRT_PUBKEYS": str(pubdir)})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "signed=false")

    def test_openwrt_secret_key_without_pubkeys_exits_2(self):
        self.assets(OPENWRT_TRIPLES)
        secret = self.tmp / "usign.sec"
        secret.write_text("not a key\n")
        r = self.openwrt("--only", "ipk", env={"OPENWRT_USIGN_SEC": str(secret)})
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("no public keys", r.stderr)
        self.assertNotIn("signed=", r.stdout)

    def test_openwrt_strict_missing_label_fails(self):
        self.assets(OPENWRT_TRIPLES)
        r = self.openwrt("--strict", "--only", "ipk", "--ephemeral-keys")
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("--strict", r.stderr)
        self.assertFalse((self.out / "pages").exists())

    def test_openwrt_apk_without_tool_exits_2(self):
        self.assets(OPENWRT_TRIPLES)
        r = self.openwrt("--ephemeral-keys", env={"APK": str(self.tmp / "no-such-apk")})
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("apk not found", r.stderr)
        self.assertFalse((self.out / "feed").exists())

    def test_openwrt_missing_sde_and_bad_arguments_exit_2(self):
        self.assertEqual(self.openwrt("--ephemeral-keys", sde=None).returncode, 2)
        self.assertEqual(self.build("openwrt-build.sh", "a", "b").returncode, 2)
        self.assertEqual(self.openwrt("--only", "deb").returncode, 2)
        self.assertEqual(self.openwrt("--bogus").returncode, 2)

    def test_openwrt_no_keys_exits_2(self):
        self.assets(OPENWRT_TRIPLES)
        r = self.openwrt("--only", "ipk")
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertNotIn("signed=", r.stdout)


if __name__ == "__main__":
    unittest.main()
