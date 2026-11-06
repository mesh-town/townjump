"""Tests of dist_tree_check.py on a tree built by dist-tree.sh from a fixture release (stdlib only)."""
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CHECK = ROOT / "scripts" / "dist-check" / "dist_tree_check.py"
V = "1.2.3"


def check(tree):
    r = subprocess.run(["python3", str(CHECK), str(tree)], capture_output=True, text=True)
    return r.returncode, r.stderr


class DistTreeCheck(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        tmp = Path(self._tmp.name)
        subprocess.run([str(ROOT / "scripts/dist/fake-release.sh"), V, str(tmp / "rel")], check=True, capture_output=True)
        subprocess.run([str(ROOT / "scripts/dist/dist-tree.sh"), str(tmp / "rel"), str(tmp / "tree"), V],
                       check=True, capture_output=True)
        self.tree = tmp / "tree"

    def test_good_tree_passes(self):
        self.assertEqual(check(self.tree)[0], 0)

    def test_missing_file(self):
        (self.tree / "linux/townjump-linux-mips.tar.gz").unlink()
        rc, err = check(self.tree)
        self.assertEqual(rc, 1)
        self.assertIn("missing: linux/townjump-linux-mips.tar.gz", err)

    def test_checksum_mismatch(self):
        with (self.tree / "windows/townjump.exe").open("ab") as f:
            f.write(b"x")
        rc, err = check(self.tree)
        self.assertEqual(rc, 1)
        self.assertIn("checksum mismatch: windows/townjump.exe", err)

    def test_version_in_file_name(self):
        (self.tree / f"linux/townjump-{V}.tar.gz").write_bytes(b"x")
        rc, err = check(self.tree)
        self.assertEqual(rc, 1)
        self.assertIn("version in file name", err)

    def test_unexpected_file(self):
        (self.tree / "macos/extra").write_bytes(b"x")
        rc, err = check(self.tree)
        self.assertEqual(rc, 1)
        self.assertIn("unexpected file: macos/extra", err)
        self.assertIn("not in SHA256SUMS: macos/extra", err)

    def test_debian_dir_is_allowed(self):
        d = self.tree / "linux/debian/dists/stable"
        d.mkdir(parents=True)
        (d / "InRelease").write_text("x")
        self.assertEqual(check(self.tree)[0], 0)

    def test_missing_directory(self):
        self.assertEqual(check(self.tree / "nope")[0], 2)


if __name__ == "__main__":
    unittest.main()
