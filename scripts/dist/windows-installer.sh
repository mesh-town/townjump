#!/usr/bin/env bash
# Builds the Windows installer with NSIS.
#   windows-installer.sh <townjump-V-x86_64-pc-windows-msvc.zip | townjump.exe> [--version <V>] [--out <dir>]
# A zip gives the version (from its name), townjump.exe and LICENSE; a bare exe needs --version and uses
# the LICENSE at the repository root. Result: <out>/townjump-setup-<V>.exe (out defaults to .).
# Exit codes: 2 usage / bad version / missing input, 1 other failure, 0 success.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
nsi="$root/packaging/windows/installer.nsi"

usage() {
  echo "usage: windows-installer.sh <zip|exe> [--version <V>] [--out <dir>]" >&2
  exit 2
}

valid_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
}

input=""
version=""
out="."
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) [[ $# -ge 2 ]] || usage; version="$2"; shift 2 ;;
    --out) [[ $# -ge 2 ]] || usage; out="$2"; shift 2 ;;
    -*) usage ;;
    *) [[ -z "$input" ]] || usage; input="$1"; shift ;;
  esac
done
[[ -n "$input" ]] || usage

if [[ ! -f "$input" ]]; then
  echo "input not found: $input" >&2
  exit 2
fi
input="$(cd "$(dirname "$input")" && pwd)/$(basename "$input")"
name="$(basename "$input")"

is_zip=0
if [[ "$name" == *.zip ]]; then
  is_zip=1
  if [[ "$name" =~ ^townjump-(.+)-x86_64-pc-windows-msvc\.zip$ ]]; then
    [[ -n "$version" ]] || version="${BASH_REMATCH[1]}"
  elif [[ -z "$version" ]]; then
    echo "zip name does not match townjump-<V>-x86_64-pc-windows-msvc.zip: $name" >&2
    exit 2
  fi
fi
if [[ -z "$version" ]]; then
  echo "a bare exe needs --version <V>" >&2
  exit 2
fi
if ! valid_version "$version"; then
  echo "bad version (want x.y.z): $version" >&2
  exit 2
fi

if ! command -v makensis >/dev/null 2>&1; then
  echo "makensis not found (apt-get install nsis / brew install makensis)" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

exe="$input"
license="$root/LICENSE"
if [[ $is_zip -eq 1 ]]; then
  if ! command -v unzip >/dev/null 2>&1; then
    echo "unzip not found" >&2
    exit 1
  fi
  mkdir "$work/zip"
  unzip -q "$input" -d "$work/zip" || { echo "cannot unpack $name" >&2; exit 1; }
  exe="$(find "$work/zip" -type f -name townjump.exe | head -n 1)"
  if [[ -z "$exe" ]]; then
    echo "townjump.exe not found in $name" >&2
    exit 1
  fi
  found="$(find "$work/zip" -type f -name LICENSE | head -n 1)"
  [[ -z "$found" ]] || license="$found"
fi
if [[ ! -f "$license" ]]; then
  echo "LICENSE not found" >&2
  exit 1
fi

target="townjump-setup-$version.exe"
makensis -V2 -WX \
  -DVERSION="$version" -DEXE="$exe" -DLICENSE="$license" -DOUTFILE="$work/$target" \
  "$nsi" >&2 || { echo "makensis failed" >&2; exit 1; }
if [[ ! -s "$work/$target" ]]; then
  echo "makensis produced no installer" >&2
  exit 1
fi

mkdir -p "$out" || exit 1
out="$(cd "$out" && pwd)"
mv "$work/$target" "$out/$target"
echo "installer: $out/$target"
