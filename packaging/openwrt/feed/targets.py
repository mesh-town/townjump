"""OpenWrt package architecture labels and the Rust targets that can serve each of them."""

AARCH64 = "aarch64-unknown-linux-musl"
X86_64 = "x86_64-unknown-linux-musl"
ARMV7_SOFT = "armv7-unknown-linux-musleabi"
ARMV7_HARD = "armv7-unknown-linux-musleabihf"
MIPS = "mips-unknown-linux-musl"
MIPSEL = "mipsel-unknown-linux-musl"

# The MIPS targets are on once the MIPS probe builds both triples; False hides every MIPS label.
MIPS_ENABLED = True

# ARMv7 labels without an FPU can only run the soft-float build; the ones with an FPU in the name may
# run the hard-float build too, but the soft-float one comes first (it runs everywhere).
_ARM_SOFT_ONLY = ("arm_cortex-a7", "arm_cortex-a9")
_ARM_WITH_FPU = (
    "arm_cortex-a5_vfpv4",
    "arm_cortex-a7_neon-vfpv4",
    "arm_cortex-a7_vfpv4",
    "arm_cortex-a8_vfpv3",
    "arm_cortex-a9_neon",
    "arm_cortex-a9_vfpv3-d16",
    "arm_cortex-a15_neon-vfpv4",
)

# Label -> triples by preference. Always the full table; use labels() and pick() to honour MIPS_ENABLED.
LABELS = {
    "aarch64_cortex-a53": [AARCH64],
    "aarch64_cortex-a72": [AARCH64],
    "aarch64_cortex-a76": [AARCH64],
    "aarch64_generic": [AARCH64],
    "x86_64": [X86_64],
    **{name: [ARMV7_SOFT] for name in _ARM_SOFT_ONLY},
    **{name: [ARMV7_SOFT, ARMV7_HARD] for name in _ARM_WITH_FPU},
    "mips_24kc": [MIPS],
    "mipsel_24kc": [MIPSEL],
    "mipsel_74kc": [MIPSEL],
    "mipsel_24kc_24kf": [MIPSEL],
}

_MIPS_TRIPLES = (MIPS, MIPSEL)


def labels() -> dict:
    """The label table without MIPS labels when MIPS_ENABLED is off."""
    if MIPS_ENABLED:
        return {k: list(v) for k, v in LABELS.items()}
    return {k: list(v) for k, v in LABELS.items() if not set(v) & set(_MIPS_TRIPLES)}


def pick(label, available_triples):
    """The most preferred triple of a label that is among available_triples; None when none is, or the label is unknown."""
    wanted = labels().get(label, [])
    return next((t for t in wanted if t in set(available_triples)), None)
