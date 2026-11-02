import contextlib
import io
import tempfile
import unittest
from pathlib import Path

import assets
import fixtures

V = "0.1.7"
T_A = "aarch64-unknown-linux-musl"
T_X = "x86_64-unknown-linux-musl"
T_R = "armv7-unknown-linux-musleabihf"


def run_main(fetched, out, triples, strict=False):
    argv = ["--fetched", str(fetched), "--out", str(out), "--version", V, "--triples", ",".join(triples)]
    if strict:
        argv.append("--strict")
    so, se = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(so), contextlib.redirect_stderr(se):
        code = assets.main(argv)
    return code, so.getvalue(), se.getvalue()


class AssetsTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.fetched = self.tmp / "fetched"
        self.out = self.tmp / "out"

    def test_lists_each_triple_path(self):
        triples = [T_A, T_X, T_R]
        fixtures.make_assets(self.fetched, V, triples)
        code, out, _ = run_main(self.fetched, self.out, triples)
        self.assertEqual(code, 0)
        got = dict(line.split("=", 1) for line in out.splitlines())
        self.assertEqual(sorted(got), sorted(triples))
        for triple, path in got.items():
            self.assertEqual(path, str(self.out / triple / "townjump"))
            self.assertTrue(Path(path).is_file())

    def test_rejects_hash_mismatch(self):
        fixtures.make_assets(self.fetched, V, [T_A], corrupt=(T_A, "hash"))
        self.assertEqual(run_main(self.fetched, self.out, [T_A])[0], 1)

    def test_rejects_archive_absent_from_sums(self):
        fixtures.make_assets(self.fetched, V, [T_A, T_X], corrupt=(T_X, "absent_from_sums"))
        code, _, err = run_main(self.fetched, self.out, [T_A])
        self.assertEqual(code, 1)
        self.assertIn("not listed", err)

    def test_rejects_wrong_version_dir(self):
        fixtures.make_assets(self.fetched, V, [T_A], corrupt=(T_A, "version_dir"))
        self.assertEqual(run_main(self.fetched, self.out, [T_A])[0], 1)

    def test_rejects_dynamic_elf(self):
        fixtures.make_assets(self.fetched, V, [T_A], corrupt=(T_A, "dynamic"))
        code, _, err = run_main(self.fetched, self.out, [T_A])
        self.assertEqual(code, 1)
        self.assertIn("PT_INTERP", err)

    def test_rejects_machine_mismatch(self):
        fixtures.make_assets(self.fetched, V, [T_A], corrupt=(T_A, "machine"))
        code, _, err = run_main(self.fetched, self.out, [T_A])
        self.assertEqual(code, 1)
        self.assertIn("expected aarch64", err)

    def test_missing_triple_strict(self):
        fixtures.make_assets(self.fetched, V, [T_A])
        self.assertEqual(run_main(self.fetched, self.out, [T_A, T_X], strict=True)[0], 2)

    def test_missing_triple_lenient_notice(self):
        fixtures.make_assets(self.fetched, V, [T_A])
        code, out, err = run_main(self.fetched, self.out, [T_A, T_X])
        self.assertEqual(code, 0)
        self.assertIn(f"notice: no archive for {T_X}", err)
        self.assertEqual(len(out.splitlines()), 1)

    def test_no_archives_at_all(self):
        fixtures.make_assets(self.fetched, V, [T_A])
        self.assertEqual(run_main(self.fetched, self.out, [T_X])[0], 1)

    def test_missing_sums_file(self):
        fixtures.make_assets(self.fetched, V, [T_A])
        (self.fetched / "SHA256SUMS").unlink()
        self.assertEqual(run_main(self.fetched, self.out, [T_A])[0], 1)

    def test_bad_arguments(self):
        self.assertEqual(run_main(self.tmp / "nope", self.out, [T_A])[0], 2)
        self.assertEqual(run_main(self.fetched, self.out, [])[0], 2)

    def test_non_regular_member_rejected(self):
        import io as _io
        import tarfile
        self.fetched.mkdir()
        top = f"townjump-{V}-{T_A}"
        path = self.fetched / f"{top}.tar.gz"
        with tarfile.open(path, "w:gz") as tar:
            info = tarfile.TarInfo(f"{top}/townjump")
            info.type, info.linkname = tarfile.SYMTYPE, "/etc/passwd"
            tar.addfile(info, _io.BytesIO())
        self.assertRaises(assets.AssetError, assets.unpack, path, V, T_A, self.out)

    def test_not_a_tarball_rejected(self):
        self.fetched.mkdir()
        path = self.fetched / "x.tar.gz"
        path.write_bytes(b"garbage")
        self.assertRaises(assets.AssetError, assets.unpack, path, V, T_A, self.out)


if __name__ == "__main__":
    unittest.main()
