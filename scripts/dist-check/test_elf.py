import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "packaging" / "openwrt" / "common"))
import fixtures  # noqa: E402
import elf  # noqa: E402

AARCH64 = "aarch64-unknown-linux-musl"
MIPSEL = "mipsel-unknown-linux-musl"
MIPS = "mips-unknown-linux-musl"


class ElfTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)

    def run_elf(self, data, triple):
        path = self.dir / "bin"
        path.write_bytes(data)
        with contextlib.redirect_stderr(io.StringIO()) as err:
            code = elf.main([str(path), "--expect", triple])
        return code, err.getvalue()

    def test_static_aarch64_ok(self):
        data = fixtures.make_elf("aarch64", "little", 64)
        self.assertEqual(self.run_elf(data, AARCH64)[0], 0)

    def test_mipsel_soft_r2_ok(self):
        data = fixtures.make_elf("mips", "little", 32, fp_abi=fixtures.FP_ABI_SOFT, isa_rev=2)
        self.assertEqual(self.run_elf(data, MIPSEL)[0], 0)

    def test_mips_big_endian_soft_r2_ok(self):
        data = fixtures.make_elf("mips", "big", 32, fp_abi=fixtures.FP_ABI_SOFT)
        self.assertEqual(self.run_elf(data, MIPS)[0], 0)

    def test_mips_hardfloat_rejected(self):
        data = fixtures.make_elf("mips", "little", 32, fp_abi=fixtures.FP_ABI_DOUBLE)
        code, err = self.run_elf(data, MIPSEL)
        self.assertEqual(code, 1)
        self.assertIn("fp_abi", err)

    def test_mips_r1_rejected(self):
        data = fixtures.make_elf("mips", "little", 32, fp_abi=fixtures.FP_ABI_SOFT, isa_rev=1)
        code, err = self.run_elf(data, MIPSEL)
        self.assertEqual(code, 1)
        self.assertIn("revision", err)

    def test_mips_without_abiflags_rejected(self):
        data = fixtures.make_elf("mips", "little", 32)
        self.assertEqual(self.run_elf(data, MIPSEL)[0], 1)

    def test_endianness_mismatch_rejected(self):
        data = fixtures.make_elf("mips", "big", 32, fp_abi=fixtures.FP_ABI_SOFT)
        code, err = self.run_elf(data, MIPSEL)
        self.assertEqual(code, 1)
        self.assertIn("byte order", err)

    def test_wrong_machine_rejected(self):
        data = fixtures.make_elf("arm", "little", 32)
        code, err = self.run_elf(data, AARCH64)
        self.assertEqual(code, 1)
        self.assertIn("class", err)

    def test_interp_rejected(self):
        data = fixtures.make_elf("aarch64", "little", 64, static=False)
        code, err = self.run_elf(data, AARCH64)
        self.assertEqual(code, 1)
        self.assertIn("PT_INTERP", err)

    def test_not_elf_rejected(self):
        self.assertEqual(self.run_elf(b"#!/bin/sh\n" * 20, AARCH64)[0], 1)

    def test_unknown_triple_exits_2(self):
        self.assertEqual(self.run_elf(b"", "riscv64-unknown-linux-musl")[0], 2)

    def test_missing_file_rejected(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(elf.main([str(self.dir / "absent"), "--expect", AARCH64]), 1)


if __name__ == "__main__":
    unittest.main()
