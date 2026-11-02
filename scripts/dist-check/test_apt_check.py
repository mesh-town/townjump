import hashlib
import io
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "packaging" / "debian"))
sys.path.insert(0, str(ROOT / "packaging" / "openwrt" / "common"))

import apt_check  # noqa: E402
import apt_repo  # noqa: E402  (only to produce input; apt_check never imports it)
import fixtures  # noqa: E402
from test_apt_repo import NOW, build_debs, make_key  # noqa: E402

HAVE_GPG = shutil.which("gpg") and shutil.which("gpgv") and shutil.which("gpgconf")


def check(*argv):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = apt_check.main([str(a) for a in argv])
    return code, out.getvalue(), err.getvalue()


class AptCheckTests(unittest.TestCase):
    def setUp(self):
        td = tempfile.TemporaryDirectory()
        self.addCleanup(td.cleanup)
        self.tmp = Path(td.name)
        self.repo, self.incoming = self.tmp / "repo", self.tmp / "incoming"
        self.pub = self.tmp / "pub.gpg"
        self.pub.write_bytes(b"stub")
        self.with_sig = False
        build_debs(self, self.incoming, {"amd64": ["0.1.7-1"], "arm64": ["0.1.7-1"], "armhf": ["0.1.7-1"]})
        if HAVE_GPG:
            secret, self.pub = make_key(self.tmp / "keys")
            self.with_sig = True
            apt_repo.update(self.repo, self.incoming, 4, secret, self.pub, now=NOW)
        else:
            from apt_support import add_to_pool, check_incoming, prune_pool
            add_to_pool(self.repo, check_incoming(self.incoming))
            apt_repo.write_dists(self.repo, prune_pool(self.repo, 4), NOW)
        self.stable = self.repo / "dists" / "stable"

    def test_good_repo_passes(self):
        code, out, err = check(self.repo, *(["--pubkey", self.pub] if self.with_sig else []))
        self.assertEqual((code, err), (0, ""))
        self.assertIn("Packages amd64: 1 entries match the pool", out)
        self.assertIn("by-hash files named by their hash", out)
        if self.with_sig:
            self.assertIn("InRelease verified", out)
            self.assertIn("Release.gpg verified", out)

    def test_stale_packages_hash(self):
        path = self.stable / "main" / "binary-amd64" / "Packages"
        path.write_bytes(path.read_bytes().replace(b"SHA256: ", b"SHA256: 00", 1))
        code, _, err = check(self.repo)
        self.assertEqual(code, 1)
        self.assertIn("FAIL", err)

    def test_pool_file_changed(self):
        pkg = next((self.repo / "pool").rglob("*_arm64.deb"))
        pkg.write_bytes(pkg.read_bytes() + b"x")
        code, _, err = check(self.repo)
        self.assertEqual(code, 1)
        self.assertIn("size of", err)

    def test_release_hash_and_gz_and_by_hash(self):
        gz = self.stable / "main" / "binary-arm64" / "Packages.gz"
        good = gz.read_bytes()
        gz.write_bytes(fixtures_gzip(b"other\n"))
        self.assertIn("Packages.gz differs", check(self.repo)[2])
        gz.write_bytes(good)
        rel = self.stable / "Release"
        text = rel.read_text()
        rel.write_text(text.replace("SHA512:\n ", "SHA512:\n 00", 1))
        self.assertIn("SHA512 of", check(self.repo)[2])
        rel.write_text(text.replace("SHA512:\n", "MD5Sum:\n", 1))
        self.assertEqual(check(self.repo)[0], 1)
        rel.write_text(text)
        victim = next(self.stable.rglob("by-hash/SHA256/*"))
        victim.rename(victim.with_name("0" * 64))
        self.assertEqual(check(self.repo)[0], 1)

    def test_missing_inrelease(self):
        if not self.with_sig:
            self.skipTest("gpg is not available")
        (self.stable / "InRelease").unlink()
        code, _, err = check(self.repo, "--pubkey", self.pub)
        self.assertEqual(code, 1)
        self.assertIn("InRelease", err)

    def test_wrong_pubkey_fails(self):
        fixtures.require_tool("gpg")
        fixtures.require_tool("gpgv")
        _, other = make_key(self.tmp / "other")
        code, _, err = check(self.repo, "--pubkey", other)
        self.assertEqual(code, 1)
        self.assertIn("gpgv rejected", err)

    def test_no_pubkey_skips_signature(self):
        code, out, _ = check(self.repo)
        self.assertEqual(code, 0)
        self.assertIn("signature: not checked", out)

    def test_bad_arguments(self):
        self.assertEqual(check()[0], 2)
        self.assertEqual(check(self.tmp / "nowhere")[0], 2)
        self.assertEqual(check(self.repo, "--pubkey", self.tmp / "none.gpg")[0], 2)

    def test_cli_runs_standalone(self):
        r = subprocess.run([sys.executable, str(HERE / "apt_check.py"), str(self.repo)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("import apt_repo", (HERE / "apt_check.py").read_text())


def fixtures_gzip(data):
    import gzip
    return gzip.compress(data)


if __name__ == "__main__":
    unittest.main()
