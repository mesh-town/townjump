#!/usr/bin/env bash
# dist-tree.sh <assets-dir> <out-dir> <version>
# Builds the direct-download tree of the dist repository (files without versions, one folder per OS) from the
# release assets in <assets-dir>, after checking every archive used against SHA256SUMS:
#   macos/ linux/ android/ windows/, SHA256SUMS (relative paths), version.txt, README.md, .nojekyll.
# linux/debian/ is written by another workflow and is never created here.
# Exit codes: 0 done, 1 missing archive or bad checksum, 2 bad arguments.
set -euo pipefail

usage() { echo "usage: dist-tree.sh <assets-dir> <out-dir> <version>" >&2; exit 2; }
[ "$#" -eq 3 ] || usage
assets="$1" out="$2" version="$3"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "dist-tree.sh: bad version: $version" >&2; exit 2; }
[ -d "$assets" ] || { echo "dist-tree.sh: no such directory: $assets" >&2; exit 2; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."

hash_of() {
  if command -v sha256sum > /dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# destination:source pairs; the source is the asset name in the release
t() { echo "townjump-$version-$1"; }
pairs=(
  "macos/townjump-macos-aarch64.tar.gz:$(t aarch64-apple-darwin).tar.gz"
  "macos/townjump-macos-x86_64.tar.gz:$(t x86_64-apple-darwin).tar.gz"
  "linux/townjump-linux-x86_64.tar.gz:$(t x86_64-unknown-linux-musl).tar.gz"
  "linux/townjump-linux-aarch64.tar.gz:$(t aarch64-unknown-linux-musl).tar.gz"
  "linux/townjump-linux-armv7hf.tar.gz:$(t armv7-unknown-linux-musleabihf).tar.gz"
  "linux/townjump-linux-armv7sf.tar.gz:$(t armv7-unknown-linux-musleabi).tar.gz"
  "linux/townjump-linux-mipsel.tar.gz:$(t mipsel-unknown-linux-musl).tar.gz"
  "linux/townjump-linux-mips.tar.gz:$(t mips-unknown-linux-musl).tar.gz"
  "android/townjump-termux-aarch64.tar.gz:$(t aarch64-linux-android).tar.gz"
  "windows/townjump-setup.exe:townjump-setup-$version.exe"
)
zip_asset="$(t x86_64-pc-windows-msvc).zip"

[ -f "$assets/SHA256SUMS" ] || { echo "dist-tree.sh: missing SHA256SUMS in $assets" >&2; exit 1; }
verify() { # <asset name>
  local name="$1" want got
  [ -f "$assets/$name" ] || { echo "dist-tree.sh: missing archive: $name" >&2; exit 1; }
  want="$(awk -v n="$name" '$2 == n { print $1 }' "$assets/SHA256SUMS")"
  [ "$(printf '%s\n' "$want" | grep -c .)" = 1 ] || { echo "dist-tree.sh: $name is not listed exactly once in SHA256SUMS" >&2; exit 1; }
  got="$(hash_of "$assets/$name")"
  [ "$got" = "$want" ] || { echo "dist-tree.sh: checksum mismatch: $name" >&2; exit 1; }
}
for p in "${pairs[@]}"; do verify "${p#*:}"; done
verify "$zip_asset"

rm -rf "$out"
mkdir -p "$out/macos" "$out/linux" "$out/android" "$out/windows"
for p in "${pairs[@]}"; do cp "$assets/${p#*:}" "$out/${p%%:*}"; done
if ! unzip -p "$assets/$zip_asset" "$(basename "${zip_asset%.zip}")/townjump.exe" > "$out/windows/townjump.exe"; then
  echo "dist-tree.sh: no townjump.exe in $zip_asset" >&2
  exit 1
fi

printf '%s\n' "$version" > "$out/version.txt"
sed "s/@VERSION@/$version/g" "$root/packaging/dist/README.md.in" > "$out/README.md"
: > "$out/.nojekyll"

: > "$out/SHA256SUMS"
while IFS= read -r f; do
  printf '%s  %s\n' "$(hash_of "$out/$f")" "$f" >> "$out/SHA256SUMS"
done < <(cd "$out" && find macos linux android windows -type f | LC_ALL=C sort)
echo "dist-tree.sh: tree for $version in $out"
