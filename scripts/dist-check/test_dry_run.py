"""Argument and exit-code checks of dry-run.sh and container.sh (no builds)."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def run(script, *args):
    return subprocess.run(["bash", str(HERE / script), *args], capture_output=True, text=True)


class DryRunArguments(unittest.TestCase):
    def test_unknown_mode_exits_2(self):
        self.assertEqual(run("dry-run.sh", "bogus").returncode, 2)

    def test_assets_dir_missing_exits_2(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = run("dry-run.sh", "deb", "--assets", str(Path(tmp) / "missing"))
        self.assertEqual(r.returncode, 2)
        self.assertIn("does not exist", r.stderr)

    def test_container_unknown_distro_exits_2(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(run("container.sh", "fedora", tmp).returncode, 2)

    def test_container_accepts_openwrt_build_but_needs_a_directory(self):
        r = run("container.sh", "openwrt-build", "/nonexistent-dry-dir")
        self.assertEqual(r.returncode, 2)
        self.assertIn("not a directory", r.stderr)

    def test_soft_float_arm_shim_is_in_armv7_soft_build(self):
        text = (HERE.parent / "dist" / "armv7-soft-build.sh").read_text()
        self.assertIn("fmaximum_num", text)
        self.assertIn("fminimum_num", text)


DIST = HERE.parent / "dist"
PAGES = HERE.parent.parent / "packaging" / "openwrt" / "common" / "publish_pages.sh"


class DistAndDebKeepEachOther(unittest.TestCase):
    """publish_pages.sh into one local bare repository, in both orders: the dist tree replaces everything but
    linux/debian, the apt part replaces only linux/debian."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        subprocess.run(["bash", str(DIST / "fake-release.sh"), "1.2.3", str(self.tmp / "rel")], check=True, capture_output=True)
        subprocess.run(["bash", str(DIST / "dist-tree.sh"), str(self.tmp / "rel"), str(self.tmp / "tree"), "1.2.3"], check=True,
                       capture_output=True)
        deb = self.tmp / "deb"
        (deb / "dists" / "stable").mkdir(parents=True)
        (deb / "dists" / "stable" / "InRelease").write_text("stub\n")
        self.repo = self.tmp / "dist.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(self.repo)], check=True)

    def publish(self, sub, src, keep=""):
        env = {**os.environ, "PAGES_KEEP": keep}
        subprocess.run(["sh", str(PAGES), sub, str(src), f"file://{self.repo}", "main", "1.2.3"], check=True, capture_output=True, env=env)

    def files(self):
        out = subprocess.run(["git", "-C", str(self.repo), "ls-tree", "-r", "--name-only", "main"], check=True, capture_output=True, text=True)
        return set(out.stdout.split())

    def assert_both(self):
        files = self.files()
        for f in ("macos/townjump-macos-aarch64.tar.gz", "linux/townjump-linux-x86_64.tar.gz", "android/townjump-termux-aarch64.tar.gz",
                  "windows/townjump.exe", "linux/debian/dists/stable/InRelease"):
            self.assertIn(f, files)

    def test_dist_then_deb_keeps_both(self):
        self.publish("-", self.tmp / "tree", keep="linux/debian")
        self.publish("linux/debian", self.tmp / "deb")
        self.assert_both()

    def test_deb_then_dist_keeps_both(self):
        self.publish("linux/debian", self.tmp / "deb")
        self.publish("-", self.tmp / "tree", keep="linux/debian")
        self.assert_both()


if __name__ == "__main__":
    unittest.main()
