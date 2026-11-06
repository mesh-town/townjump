#!/usr/bin/env bash
# keenetic-build.sh <fetched> <out> <version> <pkg-release> [--strict]
# Builds the unsigned Entware feed for Keenetic into <out>/feed and copies it to <out>/pages (the content of keenetic/
# on the Pages branch). Without --strict a missing release archive is skipped with a notice. Needs SOURCE_DATE_EPOCH.
# Exit codes: 0 done, 1 failure, 2 bad arguments or missing input.
set -euo pipefail

strict=""
if [ "$#" -eq 5 ] && [ "$5" = "--strict" ]; then
  strict="--strict"
elif [ "$#" -ne 4 ]; then
  echo "usage: keenetic-build.sh <fetched> <out> <version> <pkg-release> [--strict]" >&2
  exit 2
fi
fetched="$1" out="$2" version="$3" pkgrel="$4"
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  echo "keenetic-build.sh: SOURCE_DATE_EPOCH is required" >&2
  exit 2
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."
# shellcheck source=/dev/null
. "$root/packaging/openwrt/common/site.env"

triples="$(cd "$root/packaging/keenetic" && python3 -c 'import build_feed; print(",".join(build_feed.arches()))')"
rm -rf "${out:?}/bins" "$out/feed" "$out/pages"
mkdir -p "$out/bins"
# shellcheck disable=SC2086
listing="$(python3 "$root/packaging/openwrt/common/assets.py" --fetched "$fetched" --out "$out/bins" --version "$version" --triples "$triples" $strict)"

bins=()
while IFS= read -r line; do
  [ -n "$line" ] && bins+=(--bin "$line")
done <<< "$listing"
python3 "$root/packaging/keenetic/build_feed.py" --out "$out/feed" --version "$version" --pkg-release "$pkgrel" \
  --base-url "$KEENETIC_PAGES_URL" "${bins[@]}"
python3 "$root/scripts/dist-check/ipk_check.py" "$out/feed" --unsigned
cp -R "$out/feed" "$out/pages"
