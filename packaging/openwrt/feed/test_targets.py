"""Tests of targets.py: the label table and the choice of a Rust target per label (no external tools).

Run: python3 -m unittest discover -s packaging/openwrt/feed -t packaging/openwrt/feed
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import targets  # noqa: E402

ALL_TRIPLES = [targets.AARCH64, targets.X86_64, targets.ARMV7_SOFT, targets.ARMV7_HARD, targets.MIPS, targets.MIPSEL]
HF_LABELS = {
    "arm_cortex-a5_vfpv4", "arm_cortex-a7_neon-vfpv4", "arm_cortex-a7_vfpv4", "arm_cortex-a8_vfpv3",
    "arm_cortex-a9_neon", "arm_cortex-a9_vfpv3-d16", "arm_cortex-a15_neon-vfpv4",
}


def arm_labels():
    return {k: v for k, v in targets.LABELS.items() if k.startswith("arm_")}


class Armv7(unittest.TestCase):
    def test_soft_armv7_covers_all_nine(self):
        # given
        arm = arm_labels()
        # when
        soft_first = [k for k, v in arm.items() if v[0] == targets.ARMV7_SOFT]
        # then
        self.assertEqual(len(arm), 9)
        self.assertEqual(sorted(soft_first), sorted(arm))

    def test_hf_covers_only_fpu_labels(self):
        # given
        arm = arm_labels()
        # when
        with_hf = {k for k, v in arm.items() if targets.ARMV7_HARD in v}
        # then
        self.assertEqual(with_hf, HF_LABELS)
        self.assertNotIn("arm_cortex-a7", with_hf)
        self.assertNotIn("arm_cortex-a9", with_hf)
        self.assertTrue(all(arm[k][1] == targets.ARMV7_HARD for k in with_hf), "hard-float is second, never first")

    def test_pick_prefers_soft_over_hf(self):
        # given: both builds are available
        # when
        got = targets.pick("arm_cortex-a7_neon-vfpv4", ALL_TRIPLES)
        # then
        self.assertEqual(got, targets.ARMV7_SOFT)

    def test_pick_falls_back_to_hf_when_soft_is_missing(self):
        # given
        have = [targets.ARMV7_HARD]
        # when / then
        self.assertEqual(targets.pick("arm_cortex-a9_neon", have), targets.ARMV7_HARD)
        self.assertIsNone(targets.pick("arm_cortex-a9", have), "a label without an FPU never gets hard-float")


class Mips(unittest.TestCase):
    def setUp(self):
        self.addCleanup(setattr, targets, "MIPS_ENABLED", targets.MIPS_ENABLED)

    def test_mips_labels_follow_flag(self):
        # given
        mips = {"mips_24kc", "mipsel_24kc", "mipsel_74kc", "mipsel_24kc_24kf"}
        # when
        targets.MIPS_ENABLED = True
        on = set(targets.labels())
        picked_on = targets.pick("mipsel_74kc", ALL_TRIPLES)
        targets.MIPS_ENABLED = False
        off = set(targets.labels())
        picked_off = targets.pick("mipsel_74kc", ALL_TRIPLES)
        # then
        self.assertEqual(on - off, mips)
        self.assertEqual(len(on), 18)
        self.assertEqual(picked_on, targets.MIPSEL)
        self.assertIsNone(picked_off)

    def test_mips_labels_use_the_matching_endianness(self):
        # given / when / then
        self.assertEqual(targets.LABELS["mips_24kc"], [targets.MIPS])
        self.assertEqual(targets.LABELS["mipsel_24kc_24kf"], [targets.MIPSEL])


class Table(unittest.TestCase):
    def test_every_label_has_a_triple(self):
        # given / when / then
        for label, triples in targets.LABELS.items():
            with self.subTest(label=label):
                self.assertTrue(triples)
                self.assertLessEqual(set(triples), set(ALL_TRIPLES))

    def test_pick_none_when_nothing_available(self):
        # given / when / then
        self.assertIsNone(targets.pick("x86_64", []))
        self.assertIsNone(targets.pick("x86_64", [targets.AARCH64]))
        self.assertIsNone(targets.pick("no_such_label", ALL_TRIPLES))

    def test_no_late_or_old_labels(self):
        # given
        names = set(targets.LABELS)
        # when
        offending = [n for n in names if n.startswith(("i386", "powerpc", "riscv", "arm_arm", "arm_fa", "arm_xscale"))]
        # then
        self.assertEqual(offending, [])
        self.assertFalse(names & {"mips_mips32", "mipsel_mips32"})


if __name__ == "__main__":
    unittest.main()
