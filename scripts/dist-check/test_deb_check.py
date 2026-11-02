import gzip
import hashlib
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "packaging" / "debian"))
sys.path.insert(0, str(ROOT / "packaging" / "openwrt" / "common"))

import deb  # noqa: E402  (only to produce input; deb_check never imports it)
import deb_check  # noqa: E402
import fixtures  # noqa: E402

VERSION = "0.1.7-1"


def ar(members):
    out = b"!<arch>\n"
    for name, body in members:
        out += f"{name:<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(body):<10}`\n".encode() + body
        out += b"\n" if len(body) % 2 else b""
    return out


def unpack(gz):
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(gz)), mode="r:") as t:
        return [(m, t.extractfile(m).read() if m.isfile() else None) for m in t.getmembers()]


def pack(entries):
    bio = io.BytesIO()
    with tarfile.open(fileobj=bio, mode="w", format=tarfile.GNU_FORMAT) as t:
        for m, body in entries:
            m.size = len(body) if body is not None else 0
            t.addfile(m, io.BytesIO(body) if body is not None else None)
    return gzip.compress(bio.getvalue(), mtime=0)


class DebCheckTests(unittest.TestCase):
    def setUp(self):
        td = tempfile.TemporaryDirectory()
        self.addCleanup(td.cleanup)
        self.tmp = Path(td.name)
        for name, data in (("townjump", fixtures.make_elf("x86_64")), ("LICENSE", b"l\n"), ("README.md", b"r\n")):
            (self.tmp / name).write_bytes(data)
        self.deb = deb.build_deb(self.tmp / "townjump", VERSION, "amd64", self.tmp / "out",
                                 self.tmp / "LICENSE", self.tmp / "README.md", 1700000000)

    def rewrite(self, edit_control=None, edit_members=None):
        """Replace the deb with a modified copy; edit_control gets {name: bytes} of control.tar.gz."""
        members = deb_check.parse_ar(self.deb.read_bytes())
        if edit_control:
            entries = unpack(dict(members)["control.tar.gz"])
            bodies = {m.name: b for m, b in entries}
            edit_control(bodies)
            members = [(n, pack([(m, bodies[m.name]) for m, _ in entries]) if n == "control.tar.gz" else b)
                       for n, b in members]
        if edit_members:
            members = edit_members(members)
        self.deb.write_bytes(ar(members))

    def run_check(self, *args):
        return deb_check.main([str(self.deb), *args])

    def test_good_deb_passes(self):
        self.assertEqual(self.run_check("--arch", "amd64", "--version", VERSION), 0)

    def test_md5sums_mismatch(self):
        def edit(b):
            first = b["./md5sums"][:1]
            b["./md5sums"] = (b"1" if first == b"0" else b"0") + b["./md5sums"][1:]
        self.rewrite(edit)
        self.assertEqual(self.run_check(), 1)

    def test_installed_size_wrong(self):
        self.rewrite(lambda b: b.__setitem__("./control", b["./control"].replace(b"Installed-Size: 1", b"Installed-Size: 9")))
        self.assertEqual(self.run_check(), 1)

    def test_member_order_wrong(self):
        self.rewrite(edit_members=lambda m: [m[0], m[2], m[1]])
        self.assertEqual(self.run_check(), 1)

    def test_arch_mismatch(self):
        self.assertEqual(self.run_check("--arch", "arm64"), 1)
        self.assertEqual(self.run_check("--version", "0.1.7-2"), 1)

    def test_depends_rejected(self):
        self.rewrite(lambda b: b.__setitem__("./control", b["./control"] + b"Depends: libc6\n"))
        self.assertEqual(self.run_check(), 1)

    def test_maintainer_script_rejected(self):
        import copy

        def edit_members(members):
            out = []
            for n, b in members:
                if n == "control.tar.gz":
                    entries = unpack(b)
                    m = copy.copy(entries[0][0])
                    m.name, m.mode = "./postinst", 0o755
                    b = pack(entries + [(m, b"#!/bin/sh\n")])
                out.append((n, b))
            return out
        self.rewrite(edit_members=edit_members)
        self.assertEqual(self.run_check(), 1)

    def test_wrong_owner_rejected(self):
        def edit_members(members):
            out = []
            for n, b in members:
                if n == "data.tar.gz":
                    entries = unpack(b)
                    entries[1][0].uname = "someone"
                    b = pack(entries)
                out.append((n, b))
            return out
        self.rewrite(edit_members=edit_members)
        self.assertEqual(self.run_check(), 1)

    def test_not_ar_and_truncated(self):
        self.deb.write_bytes(b"garbage")
        self.assertEqual(self.run_check(), 1)
        self.deb.write_bytes(ar([("debian-binary", b"2.0\n")])[:-2])
        self.assertEqual(self.run_check(), 1)

    def test_wrong_file_name(self):
        renamed = self.deb.with_name("townjump_0.1.7-1_arm64.deb")
        self.deb.rename(renamed)
        self.deb = renamed
        self.assertEqual(self.run_check(), 1)

    def test_bad_arguments(self):
        self.assertEqual(deb_check.main([str(self.tmp / "none.deb")]), 2)
        self.assertEqual(self.run_check("--arch", "mips"), 2)

    def test_source_without_key_rejected(self):
        # a package built with a keyring passes, but dropping the key from md5sums and data must fail
        key = self.tmp / "k.gpg"
        key.write_bytes(b"k")
        self.deb = deb.build_deb(self.tmp / "townjump", VERSION, "arm64", self.tmp / "out2", self.tmp / "LICENSE",
                                 self.tmp / "README.md", 1, keyring=key, base_url="https://example.invalid")
        self.assertEqual(self.run_check(), 0)

        def edit_members(members):
            out = []
            for n, b in members:
                if n == "data.tar.gz":
                    b = pack([e for e in unpack(b) if not e[0].name.endswith("townjump.gpg")])
                if n == "control.tar.gz":
                    entries = unpack(b)
                    fixed = []
                    for m, body in entries:
                        if m.name == "./md5sums":
                            body = b"".join(l + b"\n" for l in body.splitlines() if b"townjump.gpg" not in l)
                        fixed.append((m, body))
                    b = pack(fixed)
                out.append((n, b))
            return out
        self.rewrite(edit_members=edit_members)
        self.assertEqual(self.run_check(), 1)


if __name__ == "__main__":
    unittest.main()
