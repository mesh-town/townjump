"""Tests of the Keenetic feed builder: layout, package contents, control fields, index, CLI errors.

Run: python3 -m unittest discover -s packaging/keenetic -t packaging/keenetic
"""
import contextlib
import gzip
import hashlib
import io
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "openwrt" / "common"))

import build_feed  # noqa: E402
import fixtures  # noqa: E402

SDE = "1760000000"
BASE = "http://feed.test"
AARCH64 = "aarch64-unknown-linux-musl"
MIPSEL = "mipsel-unknown-linux-musl"
MIPS = "mips-unknown-linux-musl"


def fields(text):
    return dict(line.split(": ", 1) for line in text.strip("\n").split("\n"))


def data_files(path):
    outer = tarfile.open(fileobj=io.BytesIO(gzip.decompress(Path(path).read_bytes())))
    inner = tarfile.open(fileobj=io.BytesIO(gzip.decompress(outer.extractfile("./data.tar.gz").read())))
    return {m.name: (stat.S_IMODE(m.mode), inner.extractfile(m).read()) for m in inner.getmembers() if m.isfile()}


def control_of(path):
    outer = tarfile.open(fileobj=io.BytesIO(gzip.decompress(Path(path).read_bytes())))
    inner = tarfile.open(fileobj=io.BytesIO(gzip.decompress(outer.extractfile("./control.tar.gz").read())))
    return inner.extractfile("./control").read().decode()


class BuildFeedTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)

    def elf(self, triple):
        path = self.tmp / ("bin-" + triple)
        path.write_bytes(fixtures.elf_for_triple(triple))
        return path

    def args(self, triples, version="0.1.7", release="1", out="feed"):
        a = ["--out", str(self.tmp / out), "--version", version, "--pkg-release", release, "--base-url", BASE + "/"]
        for t in triples:
            a += ["--bin", f"{t}={self.elf(t)}"]
        return a

    def run_main(self, argv, sde=SDE):
        env = {k: v for k, v in os.environ.items() if k != "SOURCE_DATE_EPOCH"}
        if sde is not None:
            env["SOURCE_DATE_EPOCH"] = sde
        err = io.StringIO()
        with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(err):
            code = build_feed.main(argv)
        return code, err.getvalue()

    def test_aarch64_dir_and_arch(self):
        code, err = self.run_main(self.args([AARCH64], release="3"))
        self.assertEqual(code, 0, err)
        d = self.tmp / "feed" / "aarch64-k3.10"
        self.assertEqual(sorted(p.name for p in d.iterdir()),
                         ["Packages", "Packages.gz", "townjump_0.1.7-r3_aarch64-3.10.ipk"])
        pkg = d / "townjump_0.1.7-r3_aarch64-3.10.ipk"
        f = fields(control_of(pkg))
        self.assertEqual((f["Package"], f["Version"], f["Architecture"]), ("townjump", "0.1.7-r3", "aarch64-3.10"))
        files = data_files(pkg)
        self.assertEqual(list(files), ["./opt/bin/townjump"])
        self.assertEqual(files["./opt/bin/townjump"][0], 0o755)
        entry = fields((d / "Packages").read_text())
        self.assertEqual(entry["SHA256sum"], hashlib.sha256(pkg.read_bytes()).hexdigest())
        self.assertEqual(int(entry["Size"]), pkg.stat().st_size)

    def test_mips_rows_follow_flag(self):
        for flag in (True, False):
            with self.subTest(flag=flag), mock.patch.object(build_feed, "MIPS_ENABLED", flag):
                known = build_feed.arches()
                self.assertEqual(AARCH64 in known, True)
                self.assertEqual(MIPSEL in known and MIPS in known, flag)
                if flag:
                    self.assertEqual(known[MIPSEL][:2], ("mipselsf-k3.4", "mipsel-3.4"))
                    self.assertEqual(known[MIPS][:2], ("mipssf-k3.4", "mips-3.4"))
                code, err = self.run_main(self.args([MIPSEL], out=f"o{flag}"))
                self.assertEqual(code, 0 if flag else 2, err)
                if flag:
                    self.assertTrue((self.tmp / f"o{flag}" / "mipselsf-k3.4").is_dir())
                else:
                    self.assertIn("known triple", err)

    def test_only_given_targets_built_and_root_files(self):
        code, err = self.run_main(self.args([AARCH64, MIPS]))
        self.assertEqual(code, 0, err)
        out = self.tmp / "feed"
        self.assertEqual(sorted(p.name for p in out.iterdir() if p.is_dir()), ["aarch64-k3.10", "mipssf-k3.4"])
        self.assertEqual(sorted(p.name for p in out.iterdir() if p.is_file()), [".nojekyll", "index.html", "install.sh"])
        self.assertTrue((out / "install.sh").stat().st_mode & stat.S_IXUSR)
        self.assertIn(f"{BASE}/install.sh", (out / "index.html").read_text())

    def test_install_sh_base_substituted(self):
        self.run_main(self.args([AARCH64]))
        text = (self.tmp / "feed" / "install.sh").read_text()
        self.assertNotIn("@BASE@", text)
        self.assertIn(f'BASE="${{TOWNJUMP_FEED:-{BASE}}}"', text)

    def test_reproducible_bytes(self):
        self.run_main(self.args([AARCH64], out="a"))
        self.run_main(self.args([AARCH64], out="b"))
        for rel in ("aarch64-k3.10/townjump_0.1.7-r1_aarch64-3.10.ipk", "aarch64-k3.10/Packages", "install.sh"):
            self.assertEqual((self.tmp / "a" / rel).read_bytes(), (self.tmp / "b" / rel).read_bytes())

    def test_missing_sde_exits_2(self):
        code, err = self.run_main(self.args([AARCH64]), sde=None)
        self.assertEqual(code, 2)
        self.assertIn("SOURCE_DATE_EPOCH", err)
        self.assertFalse((self.tmp / "feed").exists())

    def test_binary_of_another_machine_exits_2(self):
        argv = self.args([])
        argv += ["--bin", f"{AARCH64}={self.elf(MIPS)}"]
        code, err = self.run_main(argv)
        self.assertEqual(code, 2)
        self.assertIn(f"not {AARCH64}", err)

    def test_dynamic_binary_exits_2(self):
        path = self.tmp / "dyn"
        path.write_bytes(fixtures.elf_for_triple(AARCH64, static=False))
        code, err = self.run_main(self.args([]) + ["--bin", f"{AARCH64}={path}"])
        self.assertEqual(code, 2)
        self.assertIn("dynamically linked", err)

    def test_unknown_triple_and_no_binaries_exit_2(self):
        for extra in (["--bin", "x86_64-unknown-linux-musl=/nope"], ["--bin", "noequals"], []):
            with self.subTest(extra=extra):
                self.assertEqual(self.run_main(self.args([]) + extra)[0], 2)

    def test_bad_version_and_release_exit_2(self):
        for version, release in (("0.3", "1"), ("0.1.7", "0"), ("0.1.7", "x")):
            with self.subTest(version=version, release=release):
                self.assertEqual(self.run_main(self.args([AARCH64], version, release))[0], 2)

    def test_non_empty_out_exits_2(self):
        (self.tmp / "feed").mkdir()
        (self.tmp / "feed" / "old").write_text("x")
        self.assertEqual(self.run_main(self.args([AARCH64]))[0], 2)

    def test_unwritable_out_exits_1(self):
        (self.tmp / "file").write_text("x")
        argv = self.args([AARCH64], out="file/sub")
        self.assertEqual(self.run_main(argv)[0], 1)

    def test_cli_runs_as_script(self):
        env = {**os.environ, "SOURCE_DATE_EPOCH": SDE}
        done = subprocess.run([sys.executable, str(HERE / "build_feed.py"), *self.args([AARCH64])],
                              capture_output=True, text=True, env=env)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("townjump_0.1.7-r1_aarch64-3.10.ipk", done.stdout)


if __name__ == "__main__":
    unittest.main()
