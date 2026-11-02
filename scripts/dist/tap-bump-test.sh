#!/usr/bin/env bash
# Tests for tap-bump.sh against a local bare repository used as the tap. No network access.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tapbump="$here/tap-bump.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fails=0
pass() { echo "ok - $1"; }
fail() { fails=$((fails + 1)); echo "not ok - $1"; }
check() { if "${@:2}"; then pass "$1"; else fail "$1"; fi; }

ARM="townjump-1.2.3-aarch64-apple-darwin.tar.gz"
INTEL="townjump-1.2.3-x86_64-apple-darwin.tar.gz"
sums="$work/sums"
printf '%s  %s\n%s  %s\n' "$(printf 'a%.0s' $(seq 1 64))" "$ARM" "$(printf 'b%.0s' $(seq 1 64))" "$INTEL" > "$sums"

bare="$work/tap.git"
git init --quiet --bare --initial-branch=main "$bare"
seed="$work/seed"
git init --quiet --initial-branch=main "$seed"
echo seed > "$seed/README"
git -C "$seed" add README
git -C "$seed" -c user.name=seed -c user.email=seed@example.test commit --quiet -m seed
git -C "$seed" push --quiet "$bare" main

export TAP_REPO_URL="file://$bare"
count() { git -C "$bare" rev-list --count main; }

# empty token
export TAP_TOKEN=""
before="$(count)"
rc=0
"$tapbump" 1.2.3 --sums "$sums" > "$work/out" 2>&1 || rc=$?
check "empty token exits 0" test "$rc" = 0
check "empty token prints the notice" grep -qF '::notice::TAP_TOKEN is not set' "$work/out"
check "empty token adds no commit" test "$(count)" = "$before"

# bad version
rc=0
TAP_TOKEN=t "$tapbump" 1.2 --sums "$sums" > /dev/null 2>&1 || rc=$?
check "bad version exits 2" test "$rc" = 2
rc=0
"$tapbump" > /dev/null 2>&1 || rc=$?
check "no arguments exit 2" test "$rc" = 2

# success
export TAP_TOKEN=token
before="$(count)"
rc=0
"$tapbump" 1.2.3 --sums "$sums" > "$work/out" 2>&1 || rc=$?
check "bump exits 0" test "$rc" = 0
check "one new commit" test "$(count)" = $((before + 1))
check "author and committer" test "$(git -C "$bare" log -1 --format='%an <%ae>|%cn <%ce>' main)" = \
  'Mesh Town <340663027+mesh-town@users.noreply.github.com>|Mesh Town <340663027+mesh-town@users.noreply.github.com>'
check "commit message" test "$(git -C "$bare" log -1 --format=%s main)" = "townjump 1.2.3"
check "dates end with 12:00:00 +0000" test "$(git -C "$bare" log -1 --format='%ad|%cd' --date=format:'%H:%M:%S %z' main)" = '12:00:00 +0000|12:00:00 +0000'
check "formula in tap names v1.2.3" bash -c "git -C '$bare' show main:Formula/townjump.rb | grep -q 'download/v1.2.3/$ARM'"
check "token is not printed" bash -c "! grep -q token '$work/out'"

# repeat
before="$(count)"
rc=0
"$tapbump" 1.2.3 --sums "$sums" > "$work/out" 2>&1 || rc=$?
check "repeat exits 0" test "$rc" = 0
check "repeat adds no commit" test "$(count)" = "$before"
check "repeat says up to date" grep -qF 'tap is up to date' "$work/out"

# bump failure: no intel entry
printf '%s  %s\n' "$(printf 'a%.0s' $(seq 1 64))" "$ARM" > "$work/bad_sums"
before="$(count)"
rc=0
"$tapbump" 1.2.3 --sums "$work/bad_sums" > /dev/null 2>&1 || rc=$?
check "failed bump exits 1" test "$rc" = 1
check "failed bump adds no commit" test "$(count)" = "$before"

# clone failure
rc=0
TAP_REPO_URL="file://$work/nope.git" "$tapbump" 1.2.3 --sums "$sums" > /dev/null 2>&1 || rc=$?
check "clone failure exits 1" test "$rc" = 1

# missing source formula
rc=0
TAP_SOURCE_FORMULA="$work/none.rb" "$tapbump" 1.2.3 --sums "$sums" > /dev/null 2>&1 || rc=$?
check "missing source formula exits 2" test "$rc" = 2

[ "$fails" -eq 0 ] || { echo "# $fails failed"; exit 1; }
echo "# all passed"
