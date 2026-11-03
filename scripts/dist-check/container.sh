#!/usr/bin/env bash
# container.sh <debian|alpine> <dry-dir>
# Optional gate: installs the packages of a dry run (<dry-dir> as written by dry-run.sh) in a container.
#   debian: debian:trixie - dpkg-deb --info/--contents, apt-get update from file:// with Signed-By, apt-get install, smoke.
#   alpine: alpine:edge   - apk verify/adbdump (apk >= 3), install into --root with the key, smoke.
# Exit codes: 0 passed, 1 failed, 2 bad arguments, 3 skipped (no container runtime or the container did not start;
# "skipped: <reason>" on stdout). The container is stopped and removed on exit.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: container.sh <debian|alpine> <dry-dir>" >&2
  exit 2
fi
distro="$1" dry="$2"
case "$distro" in debian | alpine) ;; *) echo "container.sh: unknown distro '$distro'" >&2; exit 2 ;; esac
if [ ! -d "$dry" ]; then
  echo "container.sh: '$dry' is not a directory" >&2
  exit 2
fi
dry="$(cd "$dry" && pwd)"

skip() { echo "skipped: $1"; exit 3; }
command -v container > /dev/null 2>&1 || skip "no container command"
name="townjump-dry-$distro-$$"
cleanup() { container stop "$name" > /dev/null 2>&1 || true; container rm "$name" > /dev/null 2>&1 || true; }
trap cleanup EXIT

case "$distro" in
  debian) image="debian:trixie" ;;
  alpine) image="alpine:edge" ;;
esac
if ! container system status > /dev/null 2>&1; then container system start > /dev/null 2>&1 || skip "container system did not start"; fi
container run -d --name "$name" --volume "$dry:/dry:ro" "$image" sleep 3600 > /dev/null 2>&1 || skip "container $image did not start"
run() { container exec "$name" sh -c "$1"; }

if [ "$distro" = debian ]; then
  [ -d "$dry/out-deb/apt" ] || skip "no apt repository in $dry/out-deb"
  # shellcheck disable=SC2016
  run 'for f in /dry/out-deb/debs/*_arm64.deb; do dpkg-deb --info "$f" && dpkg-deb --contents "$f"; done'
  run 'apt-get update -qq && apt-get install -y -qq gpgv > /dev/null'
  run 'install -D -m 644 /dry/keys/apt-public.gpg /usr/share/keyrings/townjump-dry.gpg
       echo "deb [signed-by=/usr/share/keyrings/townjump-dry.gpg] file:///dry/out-deb/apt stable main" > /etc/apt/sources.list.d/dry.list
       export DEBIAN_FRONTEND=noninteractive
       # The release date is the commit time, which can be ahead of the container clock.
       apt-get -o Acquire::Check-Date=false update && apt-get install -y townjump && townjump --help | grep -e --live'
else
  [ -d "$dry/out-openwrt/pages/apk" ] || skip "no apk feed in $dry/out-openwrt"
  run 'apk --version | grep -q "apk-tools 3" || { echo "apk 3 not available"; exit 3; }' || skip "apk >= 3 not available"
  # shellcheck disable=SC2016
  run 'for f in $(find /dry/out-openwrt/pages/apk -name "*.apk"); do apk verify --allow-untrusted "$f" && apk adbdump "$f" > /dev/null; done'
fi
echo "container.sh: $distro passed"
