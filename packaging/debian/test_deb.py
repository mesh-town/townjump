import gzip
import io
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "openwrt" / "common"))

import deb  # noqa: E402
import fixtures  # noqa: E402

DEB_PY = HERE / "deb.py"
SDE = 1700000000
VERSION = "0.1.7-2"
KEY = b"\x99\x01\x0d invented key bytes\n"
LICENSE = b"licence text\n"
README = b"readme text\n"


def untar(gz):
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(gz)), mode="r:") as t:
        entries = t.getmembers()
        return entries, {e.name: t.extractfile(e).read() for e in entries if e.isfile()}


def members(raw):
    assert raw[:8] == b"!<arch>\n"
    pos, out = 8, []
    while pos < len(raw):
        head = raw[pos:pos + 60]
        size = int(head[48:58])
        out.append((head, raw[pos + 60:pos + 60 + size]))
        pos += 60 + size + size % 2
    return out


class Base(unittest.TestCase):
    def setUp(self):
        td = tempfile.TemporaryDirectory()
        self.addCleanup(td.cleanup)
        self.tmp = Path(td.name)
        self.binary = self.tmp / "townjump"
        self.binary.write_bytes(fixtures.make_elf("aarch64"))
        self.license = self.tmp / "LICENSE"
        self.license.write_bytes(LICENSE)
        self.readme = self.tmp / "README.md"
        self.readme.write_bytes(README)
        self.keyring = self.tmp / "townjump.gpg"
        self.keyring.write_bytes(KEY)

    def build(self, out="out", arch="amd64", keyring=False, version=VERSION):
        extra = dict(keyring=self.keyring, base_url="https://example.invalid/pkg/") if keyring else {}
        path = deb.build_deb(self.binary, version, arch, self.tmp / out, self.license, self.readme, SDE, **extra)
        raw = path.read_bytes()
        ms = dict((h[:16].decode().strip(), b) for h, b in members(raw))
        control, cbodies = untar(ms["control.tar.gz"])
        data, dbodies = untar(ms["data.tar.gz"])
        self.path, self.raw, self.control, self.cbodies, self.data, self.dbodies = path, raw, control, cbodies, data, dbodies
        return path

    def cli(self, *args, env=None):
        full = {k: v for k, v in os.environ.items() if k != "SOURCE_DATE_EPOCH"}
        full.update({"SOURCE_DATE_EPOCH": str(SDE)} if env is None else env)
        base = ["--binary", self.binary, "--out", self.tmp / "cli", "--license", self.license, "--readme", self.readme]
        return subprocess.run([sys.executable, str(DEB_PY), *map(str, base), *map(str, args)],
                              capture_output=True, text=True, env=full, timeout=60)


class DebTests(Base):
    def test_layout_bin_and_doc(self):
        self.build()
        modes = {e.name: e.mode for e in self.data if e.isfile()}
        self.assertEqual(modes, {"./usr/bin/townjump": 0o755, "./usr/share/doc/townjump/copyright": 0o644,
                                 "./usr/share/doc/townjump/README.md": 0o644})
        self.assertEqual(self.dbodies["./usr/share/doc/townjump/copyright"], LICENSE)
        self.assertEqual(self.path.name, f"townjump_{VERSION}_amd64.deb")

    def test_no_maintainer_scripts(self):
        self.build()
        self.assertEqual(sorted(e.name for e in self.control), [".", "./control", "./md5sums"])

    def test_control_fields(self):
        self.build()
        text = self.cbodies["./control"].decode()
        f = dict(line.split(": ", 1) for line in text.splitlines())
        self.assertEqual(f["Package"], "townjump")
        self.assertEqual(f["Version"], VERSION)
        self.assertEqual(f["Architecture"], "amd64")
        self.assertEqual(f["Maintainer"], "Mesh Town <340663027+mesh-town@users.noreply.github.com>")
        self.assertEqual(f["Section"], "net")
        self.assertEqual(f["Homepage"], "https://github.com/mesh-town/townjump")
        self.assertEqual(f["Description"],
                         "Holesail-compatible peer-to-peer tunnels: share a local port and reach it by key.")
        total = len(self.binary.read_bytes()) + len(LICENSE) + len(README)
        self.assertEqual(f["Installed-Size"], str(-(-total // 1024)))
        self.assertNotIn("Depends", text)

    def test_armhf_accepted(self):
        self.build(arch="armhf")
        self.assertIn("Architecture: armhf\n", self.cbodies["./control"].decode())
        self.assertTrue(self.path.name.endswith("_armhf.deb"))

    def test_keyring_adds_source_and_key(self):
        self.build(arch="arm64", keyring=True)
        src = self.dbodies["./etc/apt/sources.list.d/townjump.sources"].decode()
        self.assertIn("URIs: https://example.invalid/pkg/apt\n", src)
        self.assertIn("Architectures: arm64\n", src)
        self.assertIn("Signed-By: /usr/share/keyrings/townjump.gpg\n", src)
        self.assertEqual(self.dbodies["./usr/share/keyrings/townjump.gpg"], KEY)
        self.assertIn("etc/apt/sources.list.d/townjump.sources", self.cbodies["./md5sums"].decode())

    def test_no_keyring_no_source(self):
        self.build()
        self.assertFalse([n for n in self.dbodies if "apt" in n or "keyrings" in n])

    def test_reproducible_bytes(self):
        a = self.build("o1", keyring=True).read_bytes()
        b = self.build("o2", keyring=True).read_bytes()
        self.assertEqual(a, b)

    def test_tar_owner_root_and_sde_mtime(self):
        self.build(keyring=True)
        for e in self.control + self.data:
            self.assertEqual((e.uid, e.gid, e.uname, e.gname, e.mtime), (0, 0, "root", "root", SDE), e.name)
        self.assertEqual([e.name for e in self.data], sorted(e.name for e in self.data))
        for head, _ in members(self.raw):
            self.assertEqual(head[16:28].decode().strip(), str(SDE))
            self.assertEqual(head[28:34].decode().strip(), "0")
        ms = dict((h[:16].decode().strip(), b) for h, b in members(self.raw))
        self.assertEqual(ms["data.tar.gz"][4:8], b"\0\0\0\0")  # gzip mtime 0
        self.assertEqual(ms["data.tar.gz"][3] & 0x08, 0)  # no stored file name

    def test_missing_sde_exits_2(self):
        r = self.cli("--arch", "amd64", "--version", VERSION, env={})
        self.assertEqual(r.returncode, 2)
        self.assertIn("SOURCE_DATE_EPOCH", r.stderr)

    def test_unknown_arch_exits_2(self):
        r = self.cli("--arch", "mips", "--version", VERSION)
        self.assertEqual(r.returncode, 2)

    def test_keyring_without_base_url_exits_2(self):
        r = self.cli("--arch", "amd64", "--version", VERSION, "--keyring", self.keyring)
        self.assertEqual(r.returncode, 2)
        r = self.cli("--arch", "amd64", "--version", VERSION, "--base-url", "https://example.invalid")
        self.assertEqual(r.returncode, 2)

    def test_bad_version_and_missing_file_exit_2(self):
        self.assertEqual(self.cli("--arch", "amd64", "--version", "0.1.7").returncode, 2)
        self.binary.unlink()
        self.assertEqual(self.cli("--arch", "amd64", "--version", VERSION).returncode, 2)

    def test_cli_success_prints_path(self):
        r = self.cli("--arch", "arm64", "--version", VERSION)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(Path(r.stdout.strip()), self.tmp / "cli" / f"townjump_{VERSION}_arm64.deb")

    def test_dpkg_deb_info(self):
        tool = fixtures.require_tool("dpkg-deb")
        self.build(keyring=True)
        info = subprocess.run([tool, "--info", str(self.path)], capture_output=True, text=True, timeout=60)
        self.assertEqual(info.returncode, 0, info.stderr)
        self.assertIn(f"Version: {VERSION}", info.stdout)
        listing = subprocess.run([tool, "--contents", str(self.path)], capture_output=True, text=True, timeout=60)
        self.assertIn("./usr/bin/townjump", listing.stdout)


if __name__ == "__main__":
    unittest.main()
