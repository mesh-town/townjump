#!/usr/bin/env bash
# deb-build.sh <fetched> <out> <version> <pkg-release>
# Builds townjump_<version>-<pkg-release>_{amd64,arm64,armhf}.deb into <out>/debs/ from the release archives in
# <fetched> (checked against SHA256SUMS by assets.py --strict). Needs SOURCE_DATE_EPOCH.
# Apt signing: TOWNJUMP_APT_KEY (secret key file) and optionally TOWNJUMP_APT_PUBKEY (throwaway public keyring).
#   no key        -> packages without an apt source, no repository, signed=false
#   key           -> public keyring from TOWNJUMP_APT_PUBKEY, else the committed packaging/debian/keys/townjump.gpg
#                    (neither exists: exit 2); packages carry the apt source; a check repository is built in <out>/apt/
# The last stdout line is signed=true only when the committed keyring verified the repository; it is also appended to
# $GITHUB_OUTPUT when that is set. Exit codes: 0 done, 1 failure, 2 bad arguments or missing input.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: deb-build.sh <fetched> <out> <version> <pkg-release>" >&2
  exit 2
fi
fetched="$1" out="$2" version="$3" pkgrel="$4"
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  echo "deb-build.sh: SOURCE_DATE_EPOCH is required" >&2
  exit 2
fi
case "$pkgrel" in '' | *[!0-9]* | 0) echo "deb-build.sh: pkg-release must be an integer > 0" >&2; exit 2 ;; esac
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."
committed="$root/packaging/debian/keys/townjump.gpg"

key="${TOWNJUMP_APT_KEY:-}"
pub="" signed=false
if [ -n "$key" ]; then
  [ -f "$key" ] || { echo "deb-build.sh: key file '$key' not found" >&2; exit 2; }
  if [ -n "${TOWNJUMP_APT_PUBKEY:-}" ]; then
    pub="$TOWNJUMP_APT_PUBKEY"
  elif [ -f "$committed" ]; then
    pub="$committed" signed=true
  else
    echo "deb-build.sh: a signing key is set but there is no public keyring (TOWNJUMP_APT_PUBKEY or $committed)" >&2
    exit 2
  fi
  [ -f "$pub" ] || { echo "deb-build.sh: public keyring '$pub' not found" >&2; exit 2; }
fi

# shellcheck source=/dev/null
. "$root/packaging/openwrt/common/site.env"
rm -rf "${out:?}/bins" "$out/debs" "$out/apt"
mkdir -p "$out/bins" "$out/debs"
pairs="x86_64-unknown-linux-musl:amd64 aarch64-unknown-linux-musl:arm64 armv7-unknown-linux-musleabihf:armhf"
triples="" ; for p in $pairs; do triples="${triples:+$triples,}${p%:*}"; done
listing="$(python3 "$root/packaging/openwrt/common/assets.py" --fetched "$fetched" --out "$out/bins" --version "$version" --triples "$triples" --strict)"

for p in $pairs; do
  triple="${p%:*}" arch="${p#*:}"
  binary="$(printf '%s\n' "$listing" | sed -n "s|^$triple=||p")"
  args=(--binary "$binary" --arch "$arch" --version "$version-$pkgrel" --out "$out/debs" --license "$root/LICENSE" --readme "$root/README.md")
  if [ -n "$pub" ]; then args+=(--keyring "$pub" --base-url "$PAGES_BASE_URL/apt"); fi
  deb="$(python3 "$root/packaging/debian/deb.py" "${args[@]}")"
  python3 "$root/scripts/dist-check/deb_check.py" "$deb" --arch "$arch" --version "$version-$pkgrel"
done

if [ -n "$pub" ]; then
  TOWNJUMP_APT_KEY="$key" TOWNJUMP_APT_PUBKEY="$pub" "$here/deb-update.sh" "$out/apt" "$out/debs"
fi
echo "signed=$signed"
if [ -n "${GITHUB_OUTPUT:-}" ]; then echo "signed=$signed" >> "$GITHUB_OUTPUT"; fi
