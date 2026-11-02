"""Tests of adb_check.py. The samples in fixtures/ were made by the real apk tool (see fixtures/README.txt); the
broken variants are cut from them by hand here. The check does not import any builder.

Run: python3 -m unittest discover -s scripts/dist-check -t scripts/dist-check
"""
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import adb_check  # noqa: E402

SAMPLE_APK = HERE / "fixtures" / "sample.apk"
SAMPLE_INDEX = HERE / "fixtures" / "sample-packages.adb"


def plain_of(path):
    """The sample with the compression wrapper removed: ADB. + schema + blocks."""
    return adb_check.unwrap(Path(path).read_bytes())


def blocks_of(plain):
    """[(type, whole padded block bytes)] of a plain file."""
    out, pos = [], 8
    while pos < len(plain):
        word, = struct.unpack_from("<I", plain, pos)
        size = (word & 0x3FFFFFFF)
        end = pos + ((size + 7) & ~7)
        out.append((word >> 30, plain[pos:end]))
        pos = end
    return out


def deflated(plain):
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    return b"ADBd" + c.compress(plain) + c.flush()


def block(kind, payload):
    raw = struct.pack("<I", (kind << 30) + 4 + len(payload)) + payload
    return raw + b"\0" * (-len(raw) % 8)


class Case(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)

    def put(self, name, data):
        path = self.tmp / name
        path.write_bytes(data)
        return path

    def run_check(self, *args):
        return adb_check.main([str(a) for a in args])


class GoodSamples(Case):
    def test_good_apk_passes(self):
        # given / when
        code = self.run_check(SAMPLE_APK)
        # then
        self.assertEqual(code, 0)

    def test_good_index_passes(self):
        # given / when
        code = self.run_check(SAMPLE_APK, "--index", SAMPLE_INDEX, "--expect-index-signed")
        # then
        self.assertEqual(code, 0)

    def test_directory_checks_every_apk_and_packages_adb(self):
        # given: a feed directory with a good package and a broken index
        self.put("a.apk", SAMPLE_APK.read_bytes())
        self.put("packages.adb", b"garbage")
        # when
        code = self.run_check(self.tmp)
        # then
        self.assertEqual(code, 1)

    def test_plain_unwrapped_file_passes(self):
        # given: the ADB. form without a compression wrapper
        path = self.put("p.apk", plain_of(SAMPLE_APK))
        # when / then
        self.assertEqual(self.run_check(path), 0)

    def test_zstd_wrapper_passes(self):
        # given
        try:
            from compression import zstd
        except ImportError:
            self.skipTest("compression.zstd is not available")
        data = b"ADBc" + bytes([adb_check.ALG_ZSTD, 3]) + zstd.compress(plain_of(SAMPLE_APK))
        path = self.put("z.apk", data)
        # when / then
        self.assertEqual(self.run_check(path), 0)


class BrokenFiles(Case):
    def test_truncated_block_fails(self):
        # given: the plain file cut in the middle of the DATA block, then the compressed file cut
        plain = plain_of(SAMPLE_APK)
        cut_plain = self.put("a.apk", plain[:-12])
        cut_zipped = self.put("b.apk", SAMPLE_APK.read_bytes()[:-9])
        # when / then
        self.assertEqual(self.run_check(cut_plain), 1)
        self.assertEqual(self.run_check(cut_zipped), 1)

    def test_wrong_schema_fails(self):
        # given: the package schema replaced by the index one, the magic spoiled, the index schema in a package
        plain = plain_of(SAMPLE_APK)
        swapped = self.put("a.apk", deflated(plain[:4] + b"indx" + plain[8:]))
        spoiled = self.put("b.apk", deflated(b"ADB!" + plain[4:]))
        index_as_apk = self.put("c.apk", SAMPLE_INDEX.read_bytes())
        # when / then
        for path in (swapped, spoiled, index_as_apk):
            with self.subTest(path=path.name):
                self.assertEqual(self.run_check(path), 1)

    def test_index_without_sig_fails(self):
        # given: the sample index with its SIG block removed
        plain = plain_of(SAMPLE_INDEX)
        parts = blocks_of(plain)
        unsigned = self.put("packages.adb", deflated(plain[:8] + b"".join(b for k, b in parts if k != 1)))
        # when / then
        self.assertEqual(self.run_check(unsigned, "--expect-index-signed"), 1)
        self.assertEqual(self.run_check(unsigned), 0)

    def test_not_adb_fails(self):
        # given
        path = self.put("x.apk", b"this is just text, not an apk file at all\n")
        # when / then
        self.assertEqual(self.run_check(path), 1)

    def test_bad_args_exit_2(self):
        # given / when / then
        self.assertEqual(self.run_check(), 2)
        self.assertEqual(self.run_check(self.tmp / "missing.apk"), 2)
        self.assertEqual(self.run_check(SAMPLE_APK, "--index", self.tmp / "missing.adb"), 2)
        self.assertEqual(self.run_check(SAMPLE_APK, "--no-such-flag"), 2)

    def test_empty_directory_fails(self):
        # given: nothing to check is not a pass
        # when / then
        self.assertEqual(self.run_check(self.tmp), 1)


class BlockRules(Case):
    def parts(self):
        plain = plain_of(SAMPLE_APK)
        return plain[:8], dict((k, b) for k, b in blocks_of(plain))

    def check(self, *chunks):
        head, _ = self.parts()
        return self.run_check(self.put("t.apk", deflated(head + b"".join(chunks))))

    def test_blocks_out_of_order_fail(self):
        # given: DATA before ADB, then a SIG after DATA
        _, p = self.parts()
        # when / then
        self.assertEqual(self.check(p[2], p[0]), 1)
        self.assertEqual(self.check(p[0], p[2], p[1]), 1)

    def test_package_without_data_fails(self):
        # given
        _, p = self.parts()
        # when / then
        self.assertEqual(self.check(p[0], p[1]), 1)

    def test_bad_sig_block_fails(self):
        # given: wrong hash algorithm, and a signature that is too short
        _, p = self.parts()
        sig = p[1][4:97]
        wrong_alg = block(1, sig[:1] + b"\x02" + sig[2:])
        short = block(1, sig[:10])
        # when / then
        self.assertEqual(self.check(p[0], wrong_alg, p[2]), 1)
        self.assertEqual(self.check(p[0], short, p[2]), 1)

    def test_bad_adb_block_fails(self):
        # given: compat version 1, a root that points outside the block, a block that is too small
        _, p = self.parts()
        body = bytearray(p[0][4:])
        compat = bytearray(body)
        compat[0] = 1
        far = bytearray(body)
        far[4:8] = struct.pack("<I", 0xE0000000 + 0xFFFFF)
        # when / then
        for payload in (bytes(compat), bytes(far), b"\0" * 8):
            with self.subTest(size=len(payload)):
                self.assertEqual(self.check(block(0, payload), p[2]), 1)

    def test_unknown_wrapper_and_algorithm_fail(self):
        # given
        plain = plain_of(SAMPLE_APK)
        # when / then
        self.assertEqual(self.run_check(self.put("a.apk", b"ADBx" + plain)), 1)
        self.assertEqual(self.run_check(self.put("b.apk", b"ADBc" + bytes([9, 1]) + plain)), 1)
        self.assertEqual(self.run_check(self.put("c.apk", b"ADBc\x01")), 1)


if __name__ == "__main__":
    unittest.main()
