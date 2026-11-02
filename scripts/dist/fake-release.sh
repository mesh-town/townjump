#!/usr/bin/env bash
# Build a fixture release directory: fake-release.sh <version> <out>
# Creates 6 target archives, the source tarball, a setup stub and SHA256SUMS.
# Exit codes: 2 usage / bad version, 1 other failure, 0 success.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -ne 2 ]]; then
  echo "usage: fake-release.sh <version> <out>" >&2
  exit 2
fi
v="$1"
out="$2"
if [[ ! "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "bad version: $v" >&2
  exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/fakerel.XXXXXX")"
trap 'rm -rf "$work"' EXIT
stage="$work/out"
mkdir -p "$stage"

targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  x86_64-unknown-linux-musl
  aarch64-unknown-linux-musl
  armv7-unknown-linux-musleabihf
  x86_64-pc-windows-msvc
)

for t in "${targets[@]}"; do
  base="townjump-$v-$t"
  d="$work/$base"
  mkdir -p "$d"
  if [[ "$t" == *windows* ]]; then
    printf 'MZ stub for %s\n' "$t" >"$d/townjump.exe"
  else
    printf '#!/bin/sh\necho "Holesail CLI (stub)"\n' >"$d/townjump"
    chmod 755 "$d/townjump"
  fi
  echo "stub license" >"$d/LICENSE"
  echo "stub readme $v" >"$d/README.md"
  if [[ "$t" == *windows* ]]; then
    (cd "$work" && zip -qr "$stage/$base.zip" "$base")
  else
    tar -C "$work" -czf "$stage/$base.tar.gz" "$base"
  fi
done

src="$work/townjump-$v"
mkdir -p "$src"
echo "source stub $v" >"$src/README.md"
tar -C "$work" -czf "$stage/townjump-$v.tar.gz" "townjump-$v"
printf 'MZ setup stub %s\n' "$v" >"$stage/townjump-setup-$v.exe"

"$here/publish.sh" sums "$stage"

rm -rf "$out"
mkdir -p "$(dirname "$out")"
mv "$stage" "$out"
