import os
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "dist" / "armv7-soft-build.sh"
TARGET = "armv7-unknown-linux-musleabi"


def run(args, env_extra=None, drop=()):
    env = {k: v for k, v in os.environ.items() if k not in drop}
    env.update(env_extra or {})
    return subprocess.run(["bash", str(SCRIPT), *args], env=env, capture_output=True, text=True)


class Armv7SoftTests(unittest.TestCase):
    def test_bad_triple_exits_2(self):
        r = run(["x86_64-unknown-linux-musl"], {"CARGO_TARGET_DIR": "/nonexistent"})
        self.assertEqual(r.returncode, 2)

    def test_missing_target_dir_exits_2(self):
        r = run([TARGET], drop=("CARGO_TARGET_DIR",))
        self.assertEqual(r.returncode, 2)

    def test_release_yml_soft_entry_uses_script(self):
        text = (ROOT / ".github" / "workflows" / "release.yml").read_text()
        entry = re.search(r"- target: " + TARGET + r"\n((?:            .*\n)+)", text)
        self.assertIsNotNone(entry)
        self.assertIn("soft: true", entry.group(1))
        self.assertIn("if: matrix.zig && !matrix.mips && !matrix.soft", text)
        self.assertIn("if: matrix.soft", text)
        self.assertIn('scripts/dist/armv7-soft-build.sh "${{ matrix.target }}"', text)

    def test_local_assets_uses_script_not_copy(self):
        text = (ROOT / "scripts" / "dist-check" / "local_assets.sh").read_text()
        self.assertNotIn("fmaximum_num", text)
        self.assertIn("armv7-soft-build.sh", text)


if __name__ == "__main__":
    unittest.main()
