"""Argument and exit-code checks of dry-run.sh and container.sh (no builds)."""
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


if __name__ == "__main__":
    unittest.main()
