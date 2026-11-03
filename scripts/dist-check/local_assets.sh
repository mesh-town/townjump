#!/usr/bin/env bash
# local_assets.sh <out> <version> [triple...]
# Builds release-style archives for the Linux musl targets on this machine (cargo zigbuild; MIPS through
# scripts/mips/build.sh) into <out>: townjump-<version>-<triple>.tar.gz and SHA256SUMS. Without triples all six are built.
# Needs CARGO_TARGET_DIR (exit 2 otherwise) and a SOURCE_DATE_EPOCH (default: the HEAD commit time). The archives are
# for the dry run only: local binaries carry build paths and are never published. The caller holds the machine lock.
# Exit codes: 0 done, 1 build failure, 2 bad arguments.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: local_assets.sh <out> <version> [triple...]" >&2
  exit 2
fi
out="$1" version="$2"
shift 2
if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  echo "local_assets.sh: CARGO_TARGET_DIR must be set" >&2
  exit 2
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."
all="x86_64-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf armv7-unknown-linux-musleabi mipsel-unknown-linux-musl mips-unknown-linux-musl"
triples="${*:-$all}"
for t in $triples; do
  case " $all " in *" $t "*) ;; *) echo "local_assets.sh: unknown triple '$t'" >&2; exit 2 ;; esac
done
sde="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct HEAD)}"
stamp="$(date -u -r "$sde" +%Y%m%d%H%M.%S 2>/dev/null || date -u -d "@$sde" +%Y%m%d%H%M.%S)"

build() {
  case "$1" in
    mips*) "$root/scripts/mips/build.sh" "$1" >&2 ;;
    *)
      rustup target add "$1" >&2
      (cd "$root" && cargo zigbuild --release --locked -p townjump --target "$1" >&2)
      ;;
  esac
  [ -f "$CARGO_TARGET_DIR/$1/release/townjump" ] || { echo "local_assets.sh: no binary for $1" >&2; exit 1; }
}

# Reproducible archive: fixed owner, modes and times, listed in sorted order, gzip without name or time.
pack() {
  local t="$1" dir="townjump-$version-$1" stage
  stage="$(mktemp -d)"
  mkdir "$stage/$dir"
  cp "$CARGO_TARGET_DIR/$t/release/townjump" "$stage/$dir/townjump"
  cp "$root/LICENSE" "$root/README.md" "$stage/$dir/"
  chmod 755 "$stage/$dir" "$stage/$dir/townjump"
  chmod 644 "$stage/$dir/LICENSE" "$stage/$dir/README.md"
  touch -t "$stamp" "$stage/$dir" "$stage/$dir"/*
  (
    cd "$stage"
    export COPYFILE_DISABLE=1
    printf '%s\n' "$dir" "$dir/LICENSE" "$dir/README.md" "$dir/townjump" > list
    bsdtar -cf - --no-recursion --uid 0 --gid 0 --uname root --gname root -T list | gzip -n > "$out/$dir.tar.gz"
  )
  rm -rf "$stage"
}

mkdir -p "$out"
for t in $triples; do
  build "$t"
  pack "$t"
done
(cd "$out" && shasum -a 256 townjump-*.tar.gz | LC_ALL=C sort -k2 > SHA256SUMS)
echo "local_assets.sh: archives in $out"
