#!/bin/sh
# Build townjump for 32-bit MIPS routers (static musl, soft-float) with cargo-zigbuild.
# Usage: scripts/mips/build.sh mipsel-unknown-linux-musl|mips-unknown-linux-musl
# Both targets are tier 3: there is no prebuilt std, so the standard library is built from
# source (nightly + rust-src, -Zbuild-std). zig supplies the C compiler, musl and the linker.
# Environment:
#   CARGO_TARGET_DIR  required (exit 2 when unset)
#   RUST_NIGHTLY      nightly toolchain (default: nightly-2026-10-18)
#   RUSTFLAGS         extra flags, appended to the static-linking flags set here
# Result: $CARGO_TARGET_DIR/<triple>/release/townjump, stripped and checked
# (static ELF32, EM_MIPS, byte order matching the triple).
set -eu

TARGET="${1:-}"
case "$TARGET" in
  mipsel-unknown-linux-musl) EXPECT_DATA=1; ORDER=LSB ;;
  mips-unknown-linux-musl)   EXPECT_DATA=2; ORDER=MSB ;;
  *) echo "usage: $0 mipsel-unknown-linux-musl|mips-unknown-linux-musl" >&2; exit 2 ;;
esac
if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  echo "CARGO_TARGET_DIR must be set" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
NIGHTLY="${RUST_NIGHTLY:-nightly-2026-10-18}"

if ! rustup toolchain list | grep -q "^$NIGHTLY"; then
  rustup toolchain install "$NIGHTLY" --profile minimal --component rust-src
fi

export CARGO_PROFILE_RELEASE_STRIP=symbols
# Tier 3 targets ship no C runtime objects: link statically and let zig supply crt1.o and libc.
export RUSTFLAGS="-C target-feature=+crt-static -C link-self-contained=no ${RUSTFLAGS:-}"
unset RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER
cargo "+$NIGHTLY" zigbuild -Zbuild-std=std --release --locked -p townjump --target "$TARGET"

OUT="$CARGO_TARGET_DIR/$TARGET/release/townjump"
[ -f "$OUT" ] || { echo "missing build result: $OUT" >&2; exit 1; }

# Sanity checks: a static 32-bit MIPS executable with the expected byte order.
python3 -I - "$OUT" "$EXPECT_DATA" <<'PY'
import struct, sys
path, data = sys.argv[1], int(sys.argv[2])
with open(path, "rb") as f:
    blob = f.read()
if blob[:4] != b"\x7fELF":
    sys.exit("not an ELF file")
if blob[4] != 1:
    sys.exit("not ELF32")
if blob[5] != data:
    sys.exit("wrong byte order")
e = "<" if data == 1 else ">"
machine = struct.unpack_from(e + "H", blob, 18)[0]
if machine != 8:
    sys.exit("not a MIPS binary (e_machine=%d)" % machine)
phoff = struct.unpack_from(e + "I", blob, 28)[0]
phentsize, phnum = struct.unpack_from(e + "HH", blob, 42)
for i in range(phnum):
    if struct.unpack_from(e + "I", blob, phoff + i * phentsize)[0] == 3:
        sys.exit("binary is dynamically linked (PT_INTERP)")
flags = struct.unpack_from(e + "I", blob, 36)[0]
print("elf: ELF32 MIPS, e_flags=0x%08x" % flags)
PY
echo "ok: $OUT ($(wc -c < "$OUT") bytes, $ORDER)"
