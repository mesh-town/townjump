#!/usr/bin/env bash
# Tests for bump-formula.sh on a temporary copy of the formula. No network access.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bump="$here/bump-formula.sh"
orig="$here/../../packaging/homebrew/townjump.rb"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

export BUMP_FORMULA="$work/townjump.rb"
fails=0

pass() { echo "ok - $1"; }
fail() { fails=$((fails + 1)); echo "not ok - $1"; }
check() { if "${@:2}"; then pass "$1"; else fail "$1"; fi; }

reset() { cp "$orig" "$BUMP_FORMULA"; cp "$orig" "$work/pristine.rb"; }
unchanged() { cmp -s "$BUMP_FORMULA" "$work/pristine.rb"; }

SHA_A="$(printf 'a%.0s' $(seq 1 64))"
SHA_B="ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789"
SHA_B_LOW="$(printf '%s' "$SHA_B" | tr 'A-F' 'a-f')"
SHA_C="$(printf 'c%.0s' $(seq 1 64))"
ARM="townjump-1.2.3-aarch64-apple-darwin.tar.gz"
INTEL="townjump-1.2.3-x86_64-apple-darwin.tar.gz"
BASE="https://github.com/mesh-town/townjump/releases/download/v1.2.3"

sums_ok="$work/sums_ok"
printf '%s  %s\n%s  %s\n%s  other.zip\n' "$SHA_A" "$ARM" "$SHA_B" "$INTEL" "$SHA_C" > "$sums_ok"
sums_star="$work/sums_star"
printf '%s *%s\n%s *%s\n' "$SHA_A" "$ARM" "$SHA_C" "$INTEL" > "$sums_star"

# sha_of <name>: the sha256 value on the line right after the url that names the file.
sha_of() { grep -A2 -F "/$1\"" "$BUMP_FORMULA" | grep sha256 | head -n 1 | sed 's/.*"\(.*\)"/\1/'; }
urls() { grep -c '^ *url "' "$BUMP_FORMULA"; }

# expect_fail <name> <code> <args...>: bump must exit with the code, print to stderr, leave the formula untouched.
expect_fail() {
  local name="$1" want="$2" rc=0
  shift 2
  reset
  "$bump" "$@" > "$work/out" 2> "$work/err" || rc=$?
  if [ "$rc" -ne "$want" ]; then
    fail "$name (exit $rc, want $want)"
  elif [ ! -s "$work/err" ]; then
    fail "$name (empty stderr)"
  elif ! unchanged; then
    fail "$name (formula changed)"
  else
    pass "$name"
  fi
}

reset
if "$bump" 1.2.3 --sums "$sums_ok" > "$work/out" 2>&1; then
  pass "bump succeeds"
  check "three urls" test "$(urls)" = 3
  check "arm url" grep -qF "      url \"$BASE/$ARM\"" "$BUMP_FORMULA"
  check "intel url" grep -qF "      url \"$BASE/$INTEL\"" "$BUMP_FORMULA"
  check "sums url" grep -qF "  url \"$BASE/SHA256SUMS\"" "$BUMP_FORMULA"
  check "arm sha" test "$(sha_of "$ARM")" = "$SHA_A"
  check "uppercase hex lowered" test "$(sha_of "$INTEL")" = "$SHA_B_LOW"
  check "sums sha is the hash of the sums file" test "$(sha_of SHA256SUMS)" = "$(shasum -a 256 "$sums_ok" | awk '{ print $1 }')"
  check "placeholder lines removed" test "$(grep -c 'placeholder hash' "$BUMP_FORMULA")" = 0
  cp "$BUMP_FORMULA" "$work/after1.rb"
  "$bump" 1.2.3 --sums "$sums_ok" > /dev/null 2>&1 || true
  check "repeat leaves the file unchanged" cmp -s "$BUMP_FORMULA" "$work/after1.rb"
else
  fail "bump succeeds"
  cat "$work/out"
fi

reset
"$bump" 1.2.3 --sums "$sums_star" > /dev/null 2>&1 || true
check "star form entries" test "$(sha_of "$INTEL")" = "$SHA_C"

reset
"$bump" 1.2.3 --sums "file://$sums_ok" --base-url "https://example.test/dl/" > /dev/null 2>&1 || true
check "file:// sums go through curl" grep -qF "url \"https://example.test/dl/$ARM\"" "$BUMP_FORMULA"

# fake curl: copies the file:// path to the -o target, or fails when FAKE_CURL_FAIL is set
fakebin="$work/bin"
mkdir "$fakebin"
cat > "$fakebin/curl" << 'FAKE'
#!/usr/bin/env bash
[ -z "${FAKE_CURL_FAIL:-}" ] || exit 22
echo "$*" >> "$FAKE_CURL_LOG"
url="" out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
cp "${url#file://}" "$out"
FAKE
chmod +x "$fakebin/curl"
export FAKE_CURL_LOG="$work/curl.log"

reset
PATH="$fakebin:$PATH" "$bump" 1.2.3 --sums "file://$sums_ok" > /dev/null 2>&1 || true
check "curl called for file://" grep -qF -- "file://$sums_ok" "$FAKE_CURL_LOG"
check "fake curl result applied" test "$(sha_of "$ARM")" = "$SHA_A"

reset
TOWNJUMP_DIST_REPO=o/r "$bump" 1.2.3 --sums "$sums_ok" > /dev/null 2>&1 || true
check "TOWNJUMP_DIST_REPO changes the urls" grep -qF "url \"https://github.com/o/r/releases/download/v1.2.3/$ARM\"" "$BUMP_FORMULA"

# failures
expect_fail "version 1.2" 2 1.2 --sums "$sums_ok"
expect_fail "version v1.2.3" 2 v1.2.3 --sums "$sums_ok"
expect_fail "version 1.2.3-rc1" 2 1.2.3-rc1 --sums "$sums_ok"
expect_fail "empty version" 2 "" --sums "$sums_ok"
expect_fail "no arguments" 2
expect_fail "unknown option" 2 1.2.3 --bogus
expect_fail "--sums without value" 2 1.2.3 --sums

printf '%s  %s\n' "$SHA_A" "$ARM" > "$work/no_intel"
expect_fail "missing intel entry" 1 1.2.3 --sums "$work/no_intel"
printf '%s  %s\n%s  %s\n%s  %s\n' "$SHA_A" "$ARM" "$SHA_B" "$ARM" "$SHA_C" "$INTEL" > "$work/dup"
expect_fail "duplicate arm entry" 1 1.2.3 --sums "$work/dup"
printf '%s  %s\n%s  %s\n' "${SHA_A:1}" "$ARM" "$SHA_C" "$INTEL" > "$work/short"
expect_fail "63-char sha" 1 1.2.3 --sums "$work/short"
printf '%s  %s\n%s  %s\n' "$(printf 'z%.0s' $(seq 1 64))" "$ARM" "$SHA_C" "$INTEL" > "$work/nothex"
expect_fail "non-hex sha" 1 1.2.3 --sums "$work/nothex"
expect_fail "sums file missing" 2 1.2.3 --sums "$work/nope"

reset
rc=0
PATH="$fakebin:$PATH" FAKE_CURL_FAIL=1 "$bump" 1.2.3 --sums "https://example.test/SHA256SUMS" > /dev/null 2> "$work/err" || rc=$?
check "curl failure exits 1" test "$rc" = 1
check "curl failure leaves formula unchanged" unchanged

rc=0
BUMP_FORMULA="$work/missing.rb" "$bump" 1.2.3 --sums "$sums_ok" > /dev/null 2>&1 || rc=$?
check "missing formula exits 2" test "$rc" = 2

# a formula with a wrong number of url lines is refused untouched
grep -v 'url ".*x86_64' "$orig" > "$BUMP_FORMULA"
cp "$BUMP_FORMULA" "$work/pristine.rb"
rc=0
"$bump" 1.2.3 --sums "$sums_ok" > /dev/null 2>&1 || rc=$?
check "formula with 2 urls refused" test "$rc" = 1
check "formula with 2 urls unchanged" unchanged

# metadata
desc="$(sed -n 's/^  desc "\(.*\)"$/\1/p' "$orig")"
check "homepage_is_landing" grep -qx '  homepage "https://mesh-town.github.io/townjump/"' "$orig"
check "desc_within_80_chars" test -n "$desc" -a "${#desc}" -le 80
check "desc_has_no_vpn_word" test -z "$(printf '%s' "$desc" | grep -Ei 'vpn|relay server' || true)"

[ "$fails" -eq 0 ] || { echo "# $fails failed"; exit 1; }
echo "# all passed"
