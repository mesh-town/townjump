"""Signing tests of apt_repo.py (need gpg and gpgv; TOWNJUMP_PKG_REQUIRE_TOOLS=1 turns a skip into a failure)."""
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_apt_repo import NOW, POOL, build_debs, make_env, make_key, run_main  # noqa: F401

import apt_repo
import fixtures


def gpgv(test, pubkey: Path, *files: Path, cwd=None):
    home = Path(tempfile.mkdtemp(prefix="tj-gpgv-"))
    test.addCleanup(shutil.rmtree, home, ignore_errors=True)
    return subprocess.run(["gpgv", "--homedir", str(home), "--keyring", str(pubkey), *map(str, files)],
                          capture_output=True, text=True, timeout=60, cwd=cwd)


class SignTests(unittest.TestCase):
    def setUp(self):
        fixtures.require_tool("gpg")
        fixtures.require_tool("gpgv")
        self.tmp, self.repo, self.incoming, self.pub = make_env(self)
        self.secret, self.pub = make_key(self.tmp / "keys")
        build_debs(self, self.incoming, {"amd64": ["0.1.7-1"], "arm64": ["0.1.7-1"]})
        apt_repo.update(self.repo, self.incoming, 4, self.secret, self.pub, now=NOW)
        self.stable = self.repo / "dists" / "stable"

    def test_inrelease_verifies_with_pubkey(self):
        r = gpgv(self, self.pub, self.stable / "InRelease")
        self.assertEqual(r.returncode, 0, r.stderr)
        text = (self.stable / "InRelease").read_text()
        self.assertTrue(text.startswith("-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA512\n"))
        self.assertIn("Codename: stable\n", text)

    def test_release_gpg_detached_verifies(self):
        r = gpgv(self, self.pub, self.stable / "Release.gpg", self.stable / "Release")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue((self.stable / "Release.gpg").read_text().startswith("-----BEGIN PGP SIGNATURE-----"))

    def test_tampered_release_fails_gpgv(self):
        release = self.stable / "Release"
        release.write_bytes(release.read_bytes().replace(b"Codename: stable", b"Codename: other!"))
        self.assertNotEqual(gpgv(self, self.pub, self.stable / "Release.gpg", release).returncode, 0)
        with self.assertRaisesRegex(apt_repo.RepoError, "signature check failed"):
            apt_repo.verify_release(self.stable, self.pub)

    def test_wrong_key_fails_verification(self):
        _, other = make_key(self.tmp / "other")
        with self.assertRaisesRegex(apt_repo.RepoError, "signature check failed"):
            apt_repo.verify_release(self.stable, other)

    def test_refuses_update_without_key(self):
        fresh = self.tmp / "fresh"
        missing = self.tmp / "no-such.asc"
        code, err = run_main("update", fresh, self.incoming, "--keep", 4, "--key", missing, "--pubkey", self.pub)
        self.assertEqual(code, 1)
        self.assertIn("key file not found", err)
        self.assertFalse(fresh.exists())
        self.assertEqual(run_main("update", fresh, self.incoming, "--keep", 4, "--pubkey", self.pub)[0], 2)
        junk = self.tmp / "junk.asc"
        junk.write_text("not a key")
        self.assertEqual(run_main("update", fresh, self.incoming, "--keep", 4, "--key", junk, "--pubkey", self.pub)[0], 1)

    def test_update_via_cli_and_gnupghome_removed(self):
        before = set(Path(tempfile.gettempdir()).glob("tj-gpg-*"))
        code, err = run_main("update", self.repo, self.incoming, "--keep", 4, "--key", self.secret, "--pubkey", self.pub)
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(set(Path(tempfile.gettempdir()).glob("tj-gpg-*")), before)
        self.assertTrue((self.repo / "townjump_amd64.deb").is_file())


if __name__ == "__main__":
    unittest.main()
