#!/usr/bin/env bash
# TAP tests for publish.sh and fake-release.sh. No network.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pub="$here/publish.sh"
fake="$here/fake-release.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/pubtest.XXXXXX")"
trap 'rm -rf "$work"' EXIT

n=0
fail=0
ok() { n=$((n + 1)); echo "ok $n - $1"; }
nok() { n=$((n + 1)); fail=$((fail + 1)); echo "not ok $n - $1"; }
# expect <name> <expected-rc> <actual-rc>
expect() { if [[ "$2" == "$3" ]]; then ok "$1"; else nok "$1 (rc $3, want $2)"; fi; }
rc() { set +e; "$@" >"$work/out" 2>"$work/err"; RC=$?; set -e; }

# Fake gh: records argv and GH_TOKEN.
bin="$work/bin"
mkdir -p "$bin"
cat >"$bin/gh" <<'GH'
#!/bin/sh
{ echo "ARGV: $*"; echo "GH_TOKEN=${GH_TOKEN:-}"; } >>"$GH_LOG"
GH
chmod +x "$bin/gh"
export GH_LOG="$work/gh.log"
export PATH="$bin:$PATH"
unset GITHUB_REPOSITORY TOWNJUMP_DIST_REPO DIST_TOKEN GH_TOKEN || true

fresh() { rm -rf "$work/rel"; "$fake" 1.2.3 "$work/rel" >/dev/null 2>&1; : >"$GH_LOG"; }

fresh
lines="$(wc -l <"$work/rel/SHA256SUMS" | tr -d ' ')"
expect "sums has 11 lines" 11 "$lines"
rc bash -c "cd '$work/rel' && shasum -a 256 -c SHA256SUMS"
expect "shasum -c passes" 0 "$RC"
rc "$pub" check 1.2.3 "$work/rel"
expect "check 1.2.3 passes" 0 "$RC"
if [[ "$(head -c 2 "$work/rel/townjump-setup-1.2.3.exe")" == "MZ" ]]; then ok "setup stub starts with MZ"; else nok "setup stub starts with MZ"; fi
tar -xzf "$work/rel/townjump-1.2.3-x86_64-unknown-linux-musl.tar.gz" -C "$work"
h="$("$work/townjump-1.2.3-x86_64-unknown-linux-musl/townjump" --help)"
expect "unix stub prints help" "Holesail CLI (stub)" "$h"
zl="$(unzip -l "$work/rel/townjump-1.2.3-x86_64-pc-windows-msvc.zip")"
if [[ "$zl" == *townjump.exe* ]]; then ok "zip contains townjump.exe"; else nok "zip contains townjump.exe"; fi

# sums on an empty dir and a missing dir
mkdir "$work/empty"
rc "$pub" sums "$work/empty"
expect "sums with no files fails" 1 "$RC"
if [[ ! -e "$work/empty/SHA256SUMS" ]]; then ok "no partial SHA256SUMS on failure"; else nok "no partial SHA256SUMS on failure"; fi
rc "$pub" sums "$work/nonexistent"
expect "sums on missing dir" 2 "$RC"

# fake-release usage
rc "$fake" 1.2 "$work/x"
expect "fake-release bad version" 2 "$RC"
rc "$fake"
expect "fake-release no args" 2 "$RC"
rc "$pub"
expect "publish no args" 2 "$RC"
rc "$pub" bogus
expect "publish unknown subcommand" 2 "$RC"

# check failures
fresh
grep -v 'townjump-setup-1.2.3.exe' "$work/rel/SHA256SUMS" >"$work/s" && cp "$work/s" "$work/rel/SHA256SUMS"
rc "$pub" check 1.2.3 "$work/rel"
expect "missing sums line fails" 1 "$RC"
if grep -q 'townjump-setup-1.2.3.exe' "$work/err"; then ok "stderr names missing asset"; else nok "stderr names missing asset"; fi

fresh
grep -v 'townjump-1.2.3-mipsel-unknown-linux-musl.tar.gz' "$work/rel/SHA256SUMS" >"$work/s" && cp "$work/s" "$work/rel/SHA256SUMS"
rc "$pub" check 1.2.3 "$work/rel"
expect "missing mipsel sums line fails" 1 "$RC"
if grep -q 'townjump-1.2.3-mipsel-unknown-linux-musl.tar.gz' "$work/err"; then ok "stderr names missing mipsel asset"; else nok "stderr names missing mipsel asset"; fi

fresh
rm "$work/rel/townjump-1.2.3-mipsel-unknown-linux-musl.tar.gz"
rc "$pub" release 1.2.3 "$work/rel"
expect "release without mipsel asset fails" 1 "$RC"
if grep -q 'townjump-1.2.3-mipsel-unknown-linux-musl.tar.gz' "$work/err"; then ok "release stderr names mipsel asset"; else nok "release stderr names mipsel asset"; fi

fresh
grep 'townjump-1.2.3-aarch64-apple-darwin.tar.gz' "$work/rel/SHA256SUMS" >"$work/dup"
cat "$work/dup" >>"$work/rel/SHA256SUMS"
rc "$pub" check 1.2.3 "$work/rel"
expect "duplicate sums line fails" 1 "$RC"

fresh
echo extra >>"$work/rel/townjump-1.2.3.tar.gz"
rc "$pub" check 1.2.3 "$work/rel"
expect "modified file fails" 1 "$RC"

fresh
rm "$work/rel/townjump-1.2.3-x86_64-apple-darwin.tar.gz"
rc "$pub" check 1.2.3 "$work/rel"
expect "missing file fails" 1 "$RC"

fresh
echo "0000  extra-file.tar.gz" >>"$work/rel/SHA256SUMS"
rc "$pub" check 1.2.3 "$work/rel"
expect "extra sums names allowed" 0 "$RC"

fresh
rm "$work/rel/SHA256SUMS"
rc "$pub" check 1.2.3 "$work/rel"
expect "missing SHA256SUMS fails" 1 "$RC"

fresh
rc "$pub" check 1.2 "$work/rel"
expect "check 1.2 is usage error" 2 "$RC"
rc "$pub" check 1.2.3 "$work/nonexistent"
expect "check on missing dir" 2 "$RC"
rc "$pub" check 1.2.3
expect "check without dir" 2 "$RC"

# release
fresh
rc env GITHUB_REPOSITORY=o/townjump "$pub" release 1.2.3 "$work/rel"
expect "release in own repo" 0 "$RC"
if grep -q -- '--verify-tag' "$GH_LOG" && grep -q -- '--repo o/townjump' "$GH_LOG"; then ok "gh got --verify-tag and --repo o/townjump"; else nok "gh got --verify-tag and --repo o/townjump"; fi

: >"$GH_LOG"
rc env TOWNJUMP_DIST_REPO=o/dist DIST_TOKEN= "$pub" release 1.2.3 "$work/rel"
expect "release without token fails" 1 "$RC"
if grep -q '::error::' "$work/err"; then ok "::error:: reported"; else nok "::error:: reported"; fi
if [[ ! -s "$GH_LOG" ]]; then ok "gh not called without token"; else nok "gh not called without token"; fi

rc env TOWNJUMP_DIST_REPO=o/dist DIST_TOKEN=t "$pub" release 1.2.3 "$work/rel"
expect "release with token" 0 "$RC"
if grep -q -- '--repo o/dist' "$GH_LOG" && ! grep -q -- '--verify-tag' "$GH_LOG"; then ok "no --verify-tag for foreign repo"; else nok "no --verify-tag for foreign repo"; fi
if grep -q 'GH_TOKEN=t' "$GH_LOG"; then ok "gh saw GH_TOKEN=t"; else nok "gh saw GH_TOKEN=t"; fi

: >"$GH_LOG"
rm "$work/rel/townjump-setup-1.2.3.exe"
rc env GITHUB_REPOSITORY=o/townjump "$pub" release 1.2.3 "$work/rel"
expect "release with red check fails" 1 "$RC"
if [[ ! -s "$GH_LOG" ]]; then ok "gh not called after red check"; else nok "gh not called after red check"; fi

rc "$pub" release 1.2 "$work/rel"
expect "release bad version" 2 "$RC"

echo "1..$n"
[[ "$fail" -eq 0 ]]
