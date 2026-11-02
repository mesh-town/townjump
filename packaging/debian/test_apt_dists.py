"""dists/stable tests of apt_repo.py: Packages, Packages.gz, Release hashes, by-hash history (no gpg needed)."""
import email.utils
import gzip
import hashlib
import unittest
from pathlib import Path

from test_apt_repo import NOW, POOL, build_debs, make_env, pool_map  # noqa: F401  (also sets sys.path)

import apt_repo
import apt_support

INDEXES = [f"main/binary-{arch}/{name}" for arch in ("amd64", "arm64", "armhf") for name in ("Packages", "Packages.gz")]


class DistsTests(unittest.TestCase):
    def setUp(self):
        self.tmp, self.repo, self.incoming, self.pub = make_env(self)
        self.stable = self.repo / "dists" / "stable"
        self.clock = NOW

    def add(self, version):
        build_debs(self, self.repo / POOL, {a: [version] for a in ("amd64", "arm64", "armhf")})

    def publish(self):
        self.clock += 60
        return apt_repo.write_dists(self.repo, pool_map(self.repo), self.clock)

    def hashes(self, arch):
        d = self.stable / "main" / f"binary-{arch}"
        return {hashlib.sha256((d / n).read_bytes()).hexdigest() for n in ("Packages", "Packages.gz")}

    def stored(self, arch):
        return {p.name for p in (self.stable / "main" / f"binary-{arch}" / "by-hash" / "SHA256").iterdir()}

    def test_packages_gz_equals_packages(self):
        self.add("0.1.7-1")
        self.publish()
        for arch in ("amd64", "arm64", "armhf"):
            d = self.stable / "main" / f"binary-{arch}"
            self.assertEqual(gzip.decompress((d / "Packages.gz").read_bytes()), (d / "Packages").read_bytes())
            self.assertIn(f"Architecture: {arch}\n", (d / "Packages").read_text())

    def test_release_hashes_cover_indexes(self):
        self.add("0.1.7-1")
        release = self.publish()
        text = release.read_text()
        self.assertNotIn("MD5Sum", text)
        for line in ("Suite: stable", "Codename: stable", "Components: main", "Architectures: amd64 arm64 armhf",
                     "Acquire-By-Hash: yes"):
            self.assertIn(line + "\n", text)
        for label, algo in (("SHA256", hashlib.sha256), ("SHA512", hashlib.sha512)):
            block = text.split(f"{label}:\n")[1].split("SHA")[0]
            rows = [line.split() for line in block.splitlines()]
            self.assertEqual([r[2] for r in rows], INDEXES)
            for digest, size, rel in rows:
                data = (self.stable / rel).read_bytes()
                self.assertEqual((digest, int(size)), (algo(data).hexdigest(), len(data)))

    def test_by_hash_keeps_previous(self):
        self.add("0.1.7-1")
        self.publish()
        first = self.hashes("amd64")
        self.add("0.1.8-1")
        self.publish()
        second = self.hashes("amd64")
        self.assertNotEqual(first, second)
        self.assertEqual(self.stored("amd64"), first | second)
        self.add("0.1.9-1")
        self.publish()
        self.assertEqual(self.stored("amd64"), second | self.hashes("amd64"))  # the oldest one is gone

    def test_date_follows_clock_and_never_goes_back(self):
        self.add("0.1.7-1")
        release = self.publish()
        date = email.utils.formatdate(NOW + 60, usegmt=True)
        self.assertIn(f"Date: {date}\n", release.read_text())
        self.clock = NOW - 3600
        later = self.publish()
        self.assertGreater(email.utils.parsedate_to_datetime(self.date_of(later)).timestamp(), NOW + 60)

    @staticmethod
    def date_of(release: Path) -> str:
        return next(x[6:] for x in release.read_text().splitlines() if x.startswith("Date: "))

    def test_deterministic(self):
        self.add("0.1.7-1")
        self.publish()
        snap = {p.relative_to(self.stable): p.read_bytes() for p in self.stable.rglob("*") if p.is_file()}
        self.clock -= 60
        apt_repo.write_dists(self.repo, pool_map(self.repo), self.clock + 61)  # one second past the old Date
        again = {p.relative_to(self.stable): p.read_bytes() for p in self.stable.rglob("*") if p.is_file()}
        self.assertEqual(set(snap), set(again))
        self.assertEqual({k for k in snap if snap[k] != again[k]}, {Path("Release")})

    def test_unreadable_old_release_is_ignored(self):
        self.stable.mkdir(parents=True)
        (self.stable / "Release").write_bytes(b"\xff\xfe garbage")
        self.assertEqual(apt_repo.previous_hashes(self.stable), {})
        self.add("0.1.7-1")
        self.publish()
        self.assertTrue((self.stable / "Release").read_text().startswith("Origin:"))

    def test_clock_uses_source_date_epoch(self):
        import os
        old = os.environ.get("SOURCE_DATE_EPOCH")
        os.environ["SOURCE_DATE_EPOCH"] = "1700000000"
        try:
            self.assertEqual(apt_repo._clock(None), 1700000000.0)
            self.assertEqual(apt_repo._clock(5.0), 5.0)
        finally:
            os.environ.pop("SOURCE_DATE_EPOCH")
            if old is not None:
                os.environ["SOURCE_DATE_EPOCH"] = old
        self.assertGreater(apt_repo._clock(None), 1.7e9)


if __name__ == "__main__":
    unittest.main()
