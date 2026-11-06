#!/usr/bin/env bash
# Tests for container-ctx.sh on a fixture release from fake-release.sh. No network access.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ctx="$here/container-ctx.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0

pass() { echo "ok - $1"; }
fail() { fails=$((fails + 1)); echo "not ok - $1"; }

"$here/fake-release.sh" 1.2.3 "$work/rel" > /dev/null
fresh() { rm -rf "$work/r2"; cp -R "$work/rel" "$work/r2"; }

# ctx_has_three_platforms
"$ctx" "$work/rel" "$work/out" 1.2.3 > /dev/null
ok=1
for d in amd64 arm64 armv7; do [ -f "$work/out/linux/$d/townjump" ] || ok=0; done
if [ "$ok" = 1 ]; then pass ctx_has_three_platforms; else fail ctx_has_three_platforms; fi

# modes_are_0755
ok=1
for d in amd64 arm64 armv7; do
  if [ -z "$(find "$work/out/linux/$d/townjump" -perm 755)" ]; then ok=0; fi
done
if [ "$ok" = 1 ]; then pass modes_are_0755; else fail modes_are_0755; fi

# bad_checksum_fails
fresh
echo tampered >> "$work/r2/townjump-1.2.3-aarch64-unknown-linux-musl.tar.gz"
rc=0
"$ctx" "$work/r2" "$work/o2" 1.2.3 > /dev/null 2> "$work/err" || rc=$?
if [ "$rc" = 1 ] && grep -q mismatch "$work/err"; then pass bad_checksum_fails; else fail "bad_checksum_fails (rc $rc)"; fi

# missing_archive_fails
fresh
rm "$work/r2/townjump-1.2.3-armv7-unknown-linux-musleabihf.tar.gz"
rc=0
"$ctx" "$work/r2" "$work/o3" 1.2.3 > /dev/null 2> "$work/err" || rc=$?
if [ "$rc" = 1 ] && grep -q 'not found' "$work/err"; then pass missing_archive_fails; else fail "missing_archive_fails (rc $rc)"; fi

# unknown_arg_exit_2
rc=0; "$ctx" --bogus > /dev/null 2>&1 || rc=$?
rc2=0; "$ctx" "$work/rel" "$work/o4" 1.2.3 extra > /dev/null 2>&1 || rc2=$?
if [ "$rc" = 2 ] && [ "$rc2" = 2 ]; then pass unknown_arg_exit_2; else fail "unknown_arg_exit_2 ($rc $rc2)"; fi

[ "$fails" -eq 0 ] || { echo "# $fails failed"; exit 1; }
echo "# all passed"
