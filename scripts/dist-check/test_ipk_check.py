"""Tests of ipk_check.py. The feeds are built by hand here, not by the builders it must stay independent of.

Run: python3 -m unittest discover -s scripts/dist-check -t scripts/dist-check
"""
import contextlib
import gzip
import hashlib
import io
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent / "packaging" / "openwrt" / "common"))

import fixtures  # noqa: E402
import ipk_check  # noqa: E402

SDE = 1760000000


def gz(data):
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as g:
        g.write(data)
    return out.getvalue()


def tar_gz(entries, owner=("root", 0)):
    bio = io.BytesIO()
    with tarfile.open(fileobj=bio, mode="w", format=tarfile.GNU_FORMAT) as t:
        for name, data, mode in entries:
            ti = tarfile.TarInfo(name)
            ti.uname = ti.gname = owner[0]
            ti.uid = ti.gid = owner[1]
            ti.mtime, ti.mode, ti.size = SDE, mode, len(data)
            t.addfile(ti, io.BytesIO(data))
    return gz(bio.getvalue())


def make_ipk(directory, binary, label="aarch64-3.10", version="0.1.7-r1", owner=("root", 0)):
    control = (f"Package: townjump\nVersion: {version}\nArchitecture: {label}\nInstalled-Size: {len(binary)}\n"
               "Description: test\n")
    members = [("./debian-binary", b"2.0\n", 0o644),
               ("./data.tar.gz", tar_gz([("./opt/bin/townjump", binary, 0o755)], owner), 0o644),
               ("./control.tar.gz", tar_gz([("./control", control.encode(), 0o644)]), 0o644)]
    path = directory / f"townjump_{version}_{label}.ipk"
    path.write_bytes(tar_gz(members))
    return path, control


def make_feed(root):
    d = root / "aarch64-k3.10"
    d.mkdir(parents=True)
    path, control = make_ipk(d, fixtures.elf_for_triple("aarch64-unknown-linux-musl"))
    raw = path.read_bytes()
    at = control.index("Description:")
    entry = control[:at] + f"Filename: {path.name}\nSize: {len(raw)}\nSHA256sum: {hashlib.sha256(raw).hexdigest()}\n" + control[at:]
    (d / "Packages").write_text(entry)
    (d / "Packages.gz").write_bytes(gz(entry.encode()))
    return d


def run(args):
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        code = ipk_check.main([str(a) for a in args])
    return code, err.getvalue()


class IpkCheckTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.feed = self.tmp / "feed"
        self.dir = make_feed(self.feed)

    def test_good_feed_passes(self):
        self.assertEqual(run([self.feed, "--unsigned"]), (0, ""))

    def test_packages_size_mismatch(self):
        text = (self.dir / "Packages").read_text()
        size = text.split("Size: ")[1].split("\n")[0]
        bad = text.replace(f"Size: {size}", f"Size: {int(size) + 1}")
        (self.dir / "Packages").write_text(bad)
        (self.dir / "Packages.gz").write_bytes(gz(bad.encode()))
        code, err = run([self.feed, "--unsigned"])
        self.assertEqual(code, 1)
        self.assertIn("Size", err)

    def test_hash_mismatch(self):
        text = (self.dir / "Packages").read_text()
        digest = text.split("SHA256sum: ")[1].split("\n")[0]
        bad = text.replace(digest, "0" * 64)
        (self.dir / "Packages").write_text(bad)
        (self.dir / "Packages.gz").write_bytes(gz(bad.encode()))
        self.assertIn("SHA256sum", run([self.feed, "--unsigned"])[1])

    def test_gz_differs_from_packages(self):
        (self.dir / "Packages.gz").write_bytes(gz(b"other\n"))
        self.assertIn("Packages.gz differs", run([self.feed, "--unsigned"])[1])

    def test_missing_gz_and_stray_ipk_and_missing_file(self):
        (self.dir / "Packages.gz").unlink()
        stray, _ = make_ipk(self.dir, b"x", version="0.1.8-r1")
        code, err = run([self.feed, "--unsigned"])
        self.assertEqual(code, 1)
        self.assertIn("Packages.gz is missing", err)
        self.assertIn(f"{stray.name} is not in Packages", err)
        next(self.dir.glob("townjump_0.1.7*")).unlink()
        self.assertIn("is listed but absent", run([self.feed, "--unsigned"])[1])

    def test_bad_owner_in_data(self):
        pkg = next(self.dir.glob("*.ipk"))
        make_ipk(self.dir, fixtures.elf_for_triple("aarch64-unknown-linux-musl"), owner=("bob", 1000))
        self.assertEqual(pkg.exists(), True)
        text = (self.dir / "Packages").read_text()
        raw = pkg.read_bytes()
        bad = text.replace(text.split("SHA256sum: ")[1].split("\n")[0], hashlib.sha256(raw).hexdigest())
        (self.dir / "Packages").write_text(bad)
        (self.dir / "Packages.gz").write_bytes(gz(bad.encode()))
        code, err = run([self.feed, "--unsigned"])
        self.assertEqual(code, 1)
        self.assertIn("owner", err)

    def test_gzip_header_with_time_fails(self):
        pkg = next(self.dir.glob("*.ipk"))
        raw = bytearray(pkg.read_bytes())
        raw[4:8] = b"\x01\0\0\0"
        pkg.write_bytes(bytes(raw))
        self.assertIn("gzip header carries a time", run([self.feed, "--unsigned"])[1])

    def test_no_packages_file_fails(self):
        empty = self.tmp / "empty"
        empty.mkdir()
        self.assertEqual(run([empty, "--unsigned"])[0], 1)

    def test_argument_errors_exit_2(self):
        for args in ([self.feed], [self.feed, "--usign", "x"], [self.feed, "--unsigned", "--usign", "u", "--pubkey", "p"],
                     [self.tmp / "nope", "--unsigned"], []):
            with self.subTest(args=args):
                self.assertEqual(run(args)[0], 2)

    def test_unsigned_mode_accepts_no_sig(self):
        self.assertFalse((self.dir / "Packages.sig").exists())
        self.assertEqual(run([self.feed, "--unsigned"])[0], 0)

    def test_signed_mode_without_sig_fails(self):
        code, err = run([self.feed, "--usign", "/bin/true", "--pubkey", self.tmp / "k.pub"])
        self.assertEqual(code, 1)
        self.assertIn("Packages.sig is missing", err)

    def test_bad_signature_fails(self):
        usign = fixtures.require_tool("usign", "TOWNJUMP_USIGN")
        sec, pub, other = self.tmp / "a.sec", self.tmp / "a.pub", self.tmp / "b.pub"
        for s, p in ((sec, pub), (self.tmp / "b.sec", other)):
            subprocess.run([usign, "-G", "-c", "t", "-s", str(s), "-p", str(p)], check=True, capture_output=True)
        subprocess.run([usign, "-S", "-m", str(self.dir / "Packages"), "-s", str(sec)], check=True)
        self.assertEqual(run([self.feed, "--usign", usign, "--pubkey", pub])[0], 0)
        code, err = run([self.feed, "--usign", usign, "--pubkey", other])
        self.assertEqual(code, 1)
        self.assertIn("does not verify", err)


if __name__ == "__main__":
    unittest.main()
