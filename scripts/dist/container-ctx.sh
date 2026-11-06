#!/usr/bin/env bash
# container-ctx.sh <assets-dir> <out-dir> <version>
# Checks the linux musl release archives in <assets-dir> against SHA256SUMS and lays the binaries out as the
# Docker build context of packaging/container/Dockerfile:
#   <out>/linux/amd64/townjump  <- x86_64-unknown-linux-musl
#   <out>/linux/arm64/townjump  <- aarch64-unknown-linux-musl
#   <out>/linux/armv7/townjump  <- armv7-unknown-linux-musleabihf   (TARGETARCH=arm + TARGETVARIANT=v7)
# Exit codes: 0 done, 1 failure (bad checksum, missing archive), 2 bad arguments.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: container-ctx.sh <assets-dir> <out-dir> <version>" >&2
  exit 2
fi
assets="$1" out="$2" version="$3"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "container-ctx.sh: bad version '$version'" >&2; exit 2 ;;
esac
[ -d "$assets" ] || { echo "container-ctx.sh: '$assets' is not a directory" >&2; exit 2; }
[ -f "$assets/SHA256SUMS" ] || { echo "container-ctx.sh: $assets/SHA256SUMS not found" >&2; exit 1; }

sha_of() {
  if command -v sha256sum > /dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
rm -rf "${out:?}/linux"

for pair in x86_64-unknown-linux-musl:amd64 aarch64-unknown-linux-musl:arm64 armv7-unknown-linux-musleabihf:armv7; do
  triple="${pair%:*}" dir="${pair#*:}"
  name="townjump-$version-$triple.tar.gz"
  [ -f "$assets/$name" ] || { echo "container-ctx.sh: $name not found" >&2; exit 1; }
  want="$(awk -v n="$name" '{f=$2; sub(/^\*/, "", f); if (f == n) print $1}' "$assets/SHA256SUMS")"
  [ -n "$want" ] || { echo "container-ctx.sh: $name is not listed in SHA256SUMS" >&2; exit 1; }
  got="$(sha_of "$assets/$name")"
  [ "$got" = "$want" ] || { echo "container-ctx.sh: checksum mismatch for $name" >&2; exit 1; }
  mkdir -p "$work/$dir" "$out/linux/$dir"
  tar -xzf "$assets/$name" -C "$work/$dir" "townjump-$version-$triple/townjump"
  install -m 0755 "$work/$dir/townjump-$version-$triple/townjump" "$out/linux/$dir/townjump"
done
echo "container context ready in $out"
