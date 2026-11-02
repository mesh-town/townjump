"""Tests of ipk.py: the .ipk package and the opkg index.

Run: python3 -m unittest discover -s packaging/openwrt/common -t packaging/openwrt/common
"""
import gzip
import hashlib
import io
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

import fixtures
import ipk

SDE = 1760000000
MAINTAINER = "Mesh Town <340663027+mesh-town@users.noreply.github.com>"


def unpack_ipk(path):
    """(outer members [(TarInfo, bytes)], control text, data members [(TarInfo, bytes)])."""
    def members(raw):
        tar = tarfile.open(fileobj=io.BytesIO(gzip.decompress(raw)))
        return [(m, tar.extractfile(m).read() if m.isfile() else None) for m in tar.getmembers()]
    outer = members(Path(path).read_bytes())
    blobs = {m.name: d for m, d in outer}
    control = {m.name: d for m, d in members(blobs["./control.tar.gz"]) if d is not None}
    return outer, control["./control"].decode(), members(blobs["./data.tar.gz"])


def fields(text):
    return dict(line.split(": ", 1) for line in text.strip("\n").split("\n"))


class IpkTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.binary = self.tmp / "townjump"
        self.binary.write_bytes(fixtures.elf_for_triple("aarch64-unknown-linux-musl"))

    def build(self, dest="usr/bin/townjump", label="aarch64_generic", version="0.1.7-r1", out="out"):
        return ipk.build_ipk(self.binary, dest, label, version, self.tmp / out, SDE)

    def test_ipk_outer_and_members(self):
        path = self.build()
        outer, _, data = unpack_ipk(path)
        self.assertEqual([m.name for m, _ in outer], ["./debian-binary", "./data.tar.gz", "./control.tar.gz"])
        self.assertEqual(outer[0][1], b"2.0\n")
        self.assertTrue(all((m.uid, m.gid, m.uname, m.gname, m.mtime) == (0, 0, "root", "root", SDE)
                            for m, _ in outer + data))
        self.assertEqual(path.name, "townjump_0.1.7-r1_aarch64_generic.ipk")
        self.assertEqual(path.read_bytes()[:4], b"\x1f\x8b\x08\x00")
        self.assertEqual(path.read_bytes()[4:8], b"\0\0\0\0")
        files = {m.name: (m.mode, d) for m, d in data if m.isfile()}
        self.assertEqual(files, {"./usr/bin/townjump": (0o755, self.binary.read_bytes())})
        self.assertEqual([m.name for m, _ in data], [".", "./usr", "./usr/bin", "./usr/bin/townjump"])

    def test_control_has_label_and_version_rN(self):
        _, control, _ = unpack_ipk(self.build(label="mipsel_24kc", version="0.2.0-r3"))
        f = fields(control)
        self.assertEqual((f["Package"], f["Architecture"], f["Version"]), ("townjump", "mipsel_24kc", "0.2.0-r3"))
        self.assertEqual(f["Maintainer"], MAINTAINER)
        self.assertEqual(f["License"], "AGPL-3.0-only")
        self.assertEqual(f["URL"], "https://github.com/mesh-town/townjump")
        self.assertEqual(f["Installed-Size"], str(self.binary.stat().st_size))
        self.assertIn("Holesail-compatible", f["Description"])
        self.assertNotIn("Depends", f)

    def test_dest_opt_bin_for_entware(self):
        for dest in ("opt/bin/townjump", "/usr/bin/townjump"):
            with self.subTest(dest=dest):
                _, _, data = unpack_ipk(self.build(dest, out="o" + dest.replace("/", "_")))
                self.assertEqual([m.name for m, _ in data if m.isfile()], ["./" + dest.lstrip("/")])

    def test_bad_dest_rejected(self):
        for dest in ("", "/", "../x", "a//b"):
            with self.subTest(dest=dest), self.assertRaises(ValueError):
                self.build(dest)

    def test_reproducible_bytes(self):
        first = self.build(out="a")
        self.binary.touch()
        second = self.build(out="b")
        ipk.write_index(first.parent)
        ipk.write_index(second.parent)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        for name in ("Packages", "Packages.gz"):
            self.assertEqual((first.parent / name).read_bytes(), (second.parent / name).read_bytes())

    def test_index_entry_matches_file(self):
        path = self.build()
        ipk.write_index(path.parent)
        entry = fields((path.parent / "Packages").read_text())
        self.assertEqual(entry["Filename"], path.name)
        self.assertEqual(int(entry["Size"]), path.stat().st_size)
        self.assertEqual(entry["SHA256sum"], hashlib.sha256(path.read_bytes()).hexdigest())
        keys = list(entry)
        self.assertLess(keys.index("SHA256sum"), keys.index("Description"))
        self.assertEqual(gzip.decompress((path.parent / "Packages.gz").read_bytes()),
                         (path.parent / "Packages").read_bytes())

    def test_index_length_avoids_usign_bug_window(self):
        for k in range(64):
            with self.subTest(k=k):
                path = self.build(version="0.1.7-r" + "1" * (k + 1), out=f"w{k}")
                ipk.write_index(path.parent)
                self.assertNotIn((64 + (path.parent / "Packages").stat().st_size) % 128, (110, 111))

    def test_index_unsigned_without_key(self):
        path = self.build()
        ipk.write_index(path.parent)
        self.assertFalse((path.parent / "Packages.sig").exists())

    def test_key_without_tool_rejected(self):
        path = self.build()
        with self.assertRaises(ValueError):
            ipk.write_index(path.parent, None, self.tmp / "sec")

    def test_index_signed_when_key(self):
        usign = fixtures.require_tool("usign", "TOWNJUMP_USIGN")
        sec, pub = self.tmp / "k.sec", self.tmp / "k.pub"
        subprocess.run([usign, "-G", "-c", "test", "-s", str(sec), "-p", str(pub)], check=True,
                       capture_output=True)
        path = self.build()
        ipk.write_index(path.parent, usign, sec)
        done = subprocess.run([usign, "-V", "-m", str(path.parent / "Packages"), "-p", str(pub)])
        self.assertEqual(done.returncode, 0)


if __name__ == "__main__":
    unittest.main()
