import gzip
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import leak_check  # noqa: E402

MAIL = "340663027+mesh-town@users.noreply.github.com"
NAME = b"mesh" + b"town"
CONTROL = f"Package: townjump\nVersion: 0.1.7-1\nMaintainer: Mesh Town <{MAIL}>\n"


def tar_bytes(entries, uname="root"):
    bio = io.BytesIO()
    with tarfile.open(fileobj=bio, mode="w", format=tarfile.GNU_FORMAT) as t:
        for name, data in entries:
            ti = tarfile.TarInfo(name)
            ti.size, ti.uname, ti.gname, ti.mode = len(data), uname, "root", 0o644
            t.addfile(ti, io.BytesIO(data))
    return gzip.compress(bio.getvalue(), mtime=0)


def ar_member(name, data):
    head = f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n".encode()
    return head + data + (b"\n" if len(data) & 1 else b"")


def make_deb(path, control=CONTROL, uname="root", body=b"\x7fELF /ho" + b"me/builder/.cargo/x"):
    members = [("debian-binary", b"2.0\n"),
               ("control.tar.gz", tar_bytes([("./control", control.encode())])),
               ("data.tar.gz", tar_bytes([("./usr/bin/townjump", body)], uname))]
    Path(path).write_bytes(b"!<arch>\n" + b"".join(ar_member(n, d) for n, d in members))


class LeakCheckTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="tj-leak-"))
        self.addCleanup(__import__("shutil").rmtree, self.dir, True)

    def run_check(self):
        return leak_check.main([str(self.dir)])

    def test_clean_metadata_passes(self):
        make_deb(self.dir / "townjump_0.1.7-1_amd64.deb")
        (self.dir / "Packages").write_text(CONTROL)
        (self.dir / "install.sh").write_text("#!/bin/sh\necho done\n")
        self.assertEqual(self.run_check(), 0)

    def test_username_in_tar_header_fails(self):
        make_deb(self.dir / "a.deb", uname="builder")
        self.assertEqual(self.run_check(), 1)

    def test_home_path_in_control_fails(self):
        make_deb(self.dir / "a.deb", control=CONTROL + "Description: built in /Us" + "ers/someone/work\n")
        self.assertEqual(self.run_check(), 1)

    def test_binary_body_ignored(self):
        make_deb(self.dir / "a.deb", body=b"\x7fELF /ho" + b"me/builder/.cargo/registry " + NAME + b" \xd0\xb0@example.com")
        self.assertEqual(self.run_check(), 0)

    def test_foreign_mail_script_and_name_fail(self):
        for text in ("Maintainer: x@example.com\n", "Description: " + "".join(chr(c) for c in (0x43f, 0x440, 0x438)) + "\n",
                     "Homepage: https://example.com/" + NAME.decode() + "\n"):
            with self.subTest(text=text):
                (self.dir / "Release").write_text(text)
                self.assertEqual(self.run_check(), 1)

    def test_ipk_outer_tar_is_scanned(self):
        outer = tar_bytes([("./debian-binary", b"2.0\n"), ("./control.tar.gz", tar_bytes([("./control", b"Package: x\n/root/b\n")])),
                           ("./data.tar.gz", tar_bytes([("./opt/bin/townjump", b"x")]))])
        (self.dir / "a.ipk").write_bytes(outer)
        self.assertEqual(self.run_check(), 1)

    def test_bad_arguments_exit_2(self):
        self.assertEqual(leak_check.main([]), 2)
        self.assertEqual(leak_check.main([str(self.dir / "missing")]), 2)


if __name__ == "__main__":
    unittest.main()
