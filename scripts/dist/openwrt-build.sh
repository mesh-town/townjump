#!/usr/bin/env bash
# openwrt-build.sh <fetched> <out> <version> <pkg-release> [--strict] [--only ipk|apk] [--ephemeral-keys]
# Builds the OpenWrt feed (opkg .ipk and apk v3 per architecture label) into <out>/feed with build_feed.py, checks it
# with the independent ipk_check.py and adb_check.py, and copies it to <out>/pages (the content of openwrt/ on the
# Pages branch). Needs SOURCE_DATE_EPOCH.
# Environment: OPENWRT_USIGN_SEC, OPENWRT_APK_SEC (secret key files); OPENWRT_PUBKEYS (directory with the public keys,
# else the committed packaging/openwrt/keys); USIGN, APK (tool paths, else PATH).
# The last stdout line is signed=true only when both indexes are signed with the secret keys and verified with the
# committed public keys (no --only, no --ephemeral-keys, OPENWRT_PUBKEYS unset); it is also appended to $GITHUB_OUTPUT.
# Exit codes: 0 done, 1 failure, 2 bad arguments or missing input (also: a secret key but no public keys).
set -euo pipefail

usage() { echo "usage: openwrt-build.sh <fetched> <out> <version> <pkg-release> [--strict] [--only ipk|apk] [--ephemeral-keys]" >&2; exit 2; }
[ "$#" -ge 4 ] || usage
fetched="$1" out="$2" version="$3" pkgrel="$4"
shift 4
flags=() only="" ephemeral=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --strict) flags+=(--strict) ;;
    --ephemeral-keys) flags+=(--ephemeral-keys) ephemeral=true ;;
    --only)
      [ "$#" -ge 2 ] || usage
      case "$2" in ipk | apk) only="$2" ;; *) usage ;; esac
      flags+=(--only "$2")
      shift ;;
    *) usage ;;
  esac
  shift
done
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  echo "openwrt-build.sh: SOURCE_DATE_EPOCH is required" >&2
  exit 2
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."
committed="$root/packaging/openwrt/keys"
# shellcheck source=/dev/null
. "$root/packaging/openwrt/common/site.env"

signed=false pubdir=""
if ! $ephemeral; then
  if [ -n "${OPENWRT_PUBKEYS:-}" ]; then
    pubdir="$OPENWRT_PUBKEYS"
  elif [ -f "$committed/townjump-usign.pub" ]; then
    pubdir="$committed" signed=true
  elif [ -n "${OPENWRT_USIGN_SEC:-}" ] || [ -n "${OPENWRT_APK_SEC:-}" ]; then
    echo "openwrt-build.sh: a signing key is set but there are no public keys (OPENWRT_PUBKEYS or $committed)" >&2
    exit 2
  fi
  [ -z "$only" ] || signed=false
  [ -z "$pubdir" ] || export OPENWRT_PUBKEYS="$pubdir"
fi

apk_bin=""
if [ "$only" != ipk ]; then
  apk_bin="${APK:-$(command -v apk || true)}"
  if [ -z "$apk_bin" ] || [ ! -x "$apk_bin" ]; then
    echo "openwrt-build.sh: apk not found (set APK=<path> or put apk on PATH); use --only ipk to skip the apk half" >&2
    exit 2
  fi
fi

rm -rf "${out:?}/feed" "$out/pages"
mkdir -p "$out"
python3 "$root/packaging/openwrt/feed/build_feed.py" --fetched "$fetched" --out "$out/feed" --version "$version" \
  --pkg-release "$pkgrel" --base-url "$PAGES_BASE_URL/openwrt" "${flags[@]}"

usign_bin="${USIGN:-$(command -v usign || true)}"
pub="$out/feed/townjump-usign.pub"
if [ "$only" != apk ]; then
  for dir in "$out"/feed/opkg/*/; do
    if [ -n "$usign_bin" ] && [ -f "$pub" ]; then
      python3 "$root/scripts/dist-check/ipk_check.py" "$dir" --usign "$usign_bin" --pubkey "$pub"
    else
      echo "notice: no usign or no public key, checking $dir without the signature" >&2
      signed=false
      python3 "$root/scripts/dist-check/ipk_check.py" "$dir" --unsigned
    fi
  done
fi
if [ "$only" != ipk ]; then
  python3 "$root/scripts/dist-check/adb_check.py" "$out/feed/apk" --expect-index-signed
fi
cp -R "$out/feed" "$out/pages"
echo "signed=$signed"
if [ -n "${GITHUB_OUTPUT:-}" ]; then echo "signed=$signed" >> "$GITHUB_OUTPUT"; fi
