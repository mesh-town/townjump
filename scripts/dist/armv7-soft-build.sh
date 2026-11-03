#!/usr/bin/env bash
# armv7-soft-build.sh <armv7-unknown-linux-musleabi>
# Builds the townjump release binary for the soft-float ARMv7 musl target with cargo zigbuild.
# Why a shim: on this target (soft-float ARM) Rust f64::max/min lower to fmaximum_num/fminimum_num, and neither
# musl nor compiler_builtins provides them there, so the link fails with "undefined symbol". A tiny C object
# with both functions is linked in. The shim is only for soft-float ARM; other targets must not use it.
# Re-check on every toolchain change (Rust, zig, cargo-zigbuild) and drop the shim once linking passes without it.
# Needs CARGO_TARGET_DIR; the result is $CARGO_TARGET_DIR/<triple>/release/townjump. Run from the repo root or anywhere.
# Exit codes: 0 done, 1 build failure, 2 bad arguments.
set -euo pipefail

target="${1:-}"
if [ "$#" -ne 1 ] || [ "$target" != "armv7-unknown-linux-musleabi" ]; then
  echo "usage: armv7-soft-build.sh armv7-unknown-linux-musleabi" >&2
  exit 2
fi
if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  echo "armv7-soft-build.sh: CARGO_TARGET_DIR must be set" >&2
  exit 2
fi
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT

cat > "$dir/shim.c" <<'CSRC'
static int is_nan(double x) { return x != x; }
double fmaximum_num(double a, double b) {
  if (is_nan(a)) return b;
  if (is_nan(b)) return a;
  if (a == 0.0 && b == 0.0) return (1.0 / a < 0.0) ? b : a;
  return a > b ? a : b;
}
double fminimum_num(double a, double b) {
  if (is_nan(a)) return b;
  if (is_nan(b)) return a;
  if (a == 0.0 && b == 0.0) return (1.0 / a < 0.0) ? a : b;
  return a < b ? a : b;
}
CSRC
zig cc -target arm-linux-musleabi -Os -c "$dir/shim.c" -o "$dir/shim.o" >&2
rustup target add "$target" >&2
(cd "$root" && RUSTFLAGS="-C link-arg=$dir/shim.o ${RUSTFLAGS:-}" cargo zigbuild --release --locked -p townjump --target "$target" >&2)
[ -f "$CARGO_TARGET_DIR/$target/release/townjump" ] || { echo "armv7-soft-build.sh: no binary for $target" >&2; exit 1; }
