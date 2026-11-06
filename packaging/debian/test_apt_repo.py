"""Pool tests of apt_repo.py; also the shared helpers of the other test_apt_* files."""
import contextlib
import io
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "openwrt" / "common"))

import apt_repo  # noqa: E402
import apt_support  # noqa: E402
import deb  # noqa: E402
import fixtures  # noqa: E402

NOW = 1_760_000_000.0
SDE = 1700000000
POOL = "pool/main/t/townjump"
STUB_KEYRING = b"\x99\x01\x0d stub keyring bytes\n"


def build_debs(test, out: Path, versions: dict, payload: bytes = b"") -> list:
    """Real small townjump_<version>_<arch>.deb files for {arch: [version, ...]}."""
    scratch = Path(tempfile.mkdtemp(prefix="aptsrc-"))
    test.addCleanup(shutil.rmtree, scratch, ignore_errors=True)
    binary, lic, readme = scratch / "townjump", scratch / "LICENSE", scratch / "README.md"
    binary.write_bytes(fixtures.make_elf("aarch64") + payload)
    lic.write_bytes(b"licence\n")
    readme.write_bytes(b"readme\n")
    return [deb.build_deb(binary, v, arch, out, lic, readme, SDE) for arch, vs in versions.items() for v in vs]


def make_env(test):
    tmp = Path(tempfile.mkdtemp(prefix="aptrepo-"))
    test.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
    repo, incoming = tmp / "repo", tmp / "incoming"
    repo.mkdir()
    incoming.mkdir()
    pub = tmp / "pub.gpg"
    pub.write_bytes(STUB_KEYRING)
    return tmp, repo, incoming, pub


def make_key(dest: Path):
    """A throwaway ed25519 key made in its own GNUPGHOME: (armored secret key, binary public keyring)."""
    fixtures.require_tool("gpg")
    fixtures.require_tool("gpgv")
    home = Path(tempfile.mkdtemp(prefix="tjk-"))
    dest.mkdir(parents=True, exist_ok=True)
    try:
        base = ["gpg", "--homedir", str(home), "--batch", "--no-tty", "--pinentry-mode", "loopback", "--passphrase", ""]
        subprocess.run(base + ["--quick-generate-key", "Test <test@example.invalid>", "ed25519", "sign", "never"],
                       check=True, capture_output=True)
        secret, public = dest / "secret.asc", dest / "townjump.gpg"
        secret.write_bytes(subprocess.run(base + ["--armor", "--export-secret-keys"], check=True,
                                          capture_output=True).stdout)
        public.write_bytes(subprocess.run(base + ["--export"], check=True, capture_output=True).stdout)
        return secret, public
    finally:
        subprocess.run(["gpgconf", "--homedir", str(home), "--kill", "gpg-agent"], capture_output=True)
        shutil.rmtree(home, ignore_errors=True)


def run_main(*argv):
    err = io.StringIO()
    with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
        code = apt_repo.main([str(a) for a in argv])
    return code, err.getvalue()


def pool_map(repo: Path) -> dict:
    return apt_support.pool_by_arch(repo)


class PoolTests(unittest.TestCase):
    def setUp(self):
        self.tmp, self.repo, self.incoming, self.pub = make_env(self)

    def stage(self, versions, wipe=True):
        if wipe:
            shutil.rmtree(self.incoming)
        return build_debs(self, self.incoming, versions)

    def add(self, versions):
        self.stage(versions)
        debs = apt_support.check_incoming(self.incoming)
        apt_support.add_to_pool(self.repo, debs)

    def test_packages_entry_matches_deb(self):
        (path,) = self.stage({"amd64": ["0.1.7-1"]})
        text = apt_support.packages_index([path]).decode()
        fields = dict(line.split(": ", 1) for line in text.splitlines() if not line.startswith(" "))
        raw = path.read_bytes()
        self.assertEqual(fields["Package"], "townjump")
        self.assertEqual(fields["Version"], "0.1.7-1")
        self.assertEqual(fields["Filename"], f"{POOL}/{path.name}")
        self.assertEqual(fields["Size"], str(len(raw)))
        self.assertEqual(fields["SHA256"], apt_support.sha(raw))
        self.assertTrue(text.endswith("\n") and not text.endswith("\n\n"))
        self.assertEqual(apt_support.packages_index([]), b"")

    def test_rejects_incoming_wrong_arch(self):
        (path,) = self.stage({"amd64": ["0.1.7-1"]})
        path.rename(self.incoming / "townjump_0.1.7-1_i386.deb")
        code, err = run_main("update", self.repo, self.incoming, "--keep", 4, "--key", self.pub, "--pubkey", self.pub)
        self.assertEqual(code, 1)
        self.assertIn("unexpected package name", err)
        self.assertFalse((self.repo / "pool").exists())
        path = self.incoming / "townjump_0.1.7-1_i386.deb"
        path.rename(self.incoming / "townjump_0.1.7_amd64.deb")  # no revision
        self.assertEqual(run_main("update", self.repo, self.incoming, "--keep", 4, "--key", self.pub,
                                  "--pubkey", self.pub)[0], 1)

    def test_rejects_control_mismatch(self):
        (path,) = self.stage({"amd64": ["0.1.7-1"]})
        path.rename(self.incoming / "townjump_0.1.8-1_amd64.deb")
        with self.assertRaisesRegex(apt_support.RepoError, "does not match"):
            apt_support.check_incoming(self.incoming)

    def test_rejects_non_deb_and_empty(self):
        (self.incoming / "townjump_0.1.7-1_amd64.deb").write_bytes(b"junk")
        with self.assertRaisesRegex(apt_support.RepoError, "not a deb"):
            apt_support.check_incoming(self.incoming)
        shutil.rmtree(self.incoming)
        self.incoming.mkdir()
        with self.assertRaisesRegex(apt_support.RepoError, "no .deb"):
            apt_support.check_incoming(self.incoming)

    def test_versions_sort_numerically(self):
        self.add({"amd64": ["0.1.9-1", "0.1.10-1", "0.1.10-2", "0.2.0-1"]})
        names = [p.name for p in pool_map(self.repo)["amd64"]]
        self.assertEqual(names, ["townjump_0.1.9-1_amd64.deb", "townjump_0.1.10-1_amd64.deb",
                                 "townjump_0.1.10-2_amd64.deb", "townjump_0.2.0-1_amd64.deb"])

    def test_keep_prunes_old_versions(self):
        self.add({"amd64": [f"0.1.{n}-1" for n in range(1, 6)], "armhf": ["0.1.1-1"]})
        pool = apt_support.prune_pool(self.repo, 4)
        self.assertEqual([p.name for p in pool["amd64"]], [f"townjump_0.1.{n}-1_amd64.deb" for n in range(2, 6)])
        self.assertEqual(len(list((self.repo / POOL).glob("*_amd64.deb"))), 4)
        self.assertEqual(len(pool["armhf"]), 1)
        with self.assertRaises(apt_support.RepoError):
            apt_support.prune_pool(self.repo, 0)

    def test_same_name_keeps_published(self):
        (first,) = self.stage({"amd64": ["0.1.7-1"]})
        apt_support.add_to_pool(self.repo, [first])
        published = (self.repo / POOL / first.name).read_bytes()
        shutil.rmtree(self.incoming)
        (second,) = build_debs(self, self.incoming, {"amd64": ["0.1.7-1"]}, payload=b"different")
        self.assertNotEqual(second.read_bytes(), published)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            apt_support.add_to_pool(self.repo, [second])
        self.assertEqual((self.repo / POOL / first.name).read_bytes(), published)
        self.assertIn("keeps the published", err.getvalue())

    def test_direct_link_is_latest_per_arch(self):
        self.add({"amd64": ["0.1.9-1", "0.1.10-1"], "arm64": ["0.1.2-1"]})
        pool = apt_support.prune_pool(self.repo, 4)
        apt_support.write_direct_links(self.repo, pool)
        newest = (self.repo / POOL / "townjump_0.1.10-1_amd64.deb").read_bytes()
        self.assertEqual((self.repo / "townjump_amd64.deb").read_bytes(), newest)
        self.assertEqual((self.repo / "townjump_arm64.deb").read_bytes(),
                         (self.repo / POOL / "townjump_0.1.2-1_arm64.deb").read_bytes())
        self.assertFalse((self.repo / "townjump_armhf.deb").exists())
        (self.repo / "townjump_armhf.deb").write_bytes(b"stale")
        apt_support.write_direct_links(self.repo, pool)
        self.assertFalse((self.repo / "townjump_armhf.deb").exists())

    def test_foreign_file_in_pool_fails(self):
        (self.repo / POOL).mkdir(parents=True)
        (self.repo / POOL / "stray.txt").write_text("x")
        with self.assertRaisesRegex(apt_support.RepoError, "unexpected file"):
            apt_support.pool_by_arch(self.repo)


if __name__ == "__main__":
    unittest.main()
