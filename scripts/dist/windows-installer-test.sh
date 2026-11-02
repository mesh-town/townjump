#!/usr/bin/env bash
# TAP tests for windows-installer.sh. Needs makensis, zip and 7zz (or 7z); no network.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/windows-installer.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

sz=""
for c in 7zz 7z; do
  if command -v "$c" >/dev/null 2>&1; then sz="$c"; break; fi
done
if [[ -z "$sz" ]] || ! command -v makensis >/dev/null 2>&1 || ! command -v zip >/dev/null 2>&1; then
  echo "Bail out! need makensis, zip and 7zz/7z"
  exit 2
fi

n=0
failed=0
ok() { n=$((n + 1)); echo "ok $n - $1"; }
nok() { n=$((n + 1)); failed=$((failed + 1)); echo "not ok $n - $1"; }
expect_rc() { # name want got
  if [[ "$3" == "$2" ]]; then ok "$1"; else nok "$1 (exit $3, want $2)"; fi
}
run() { # sets rc, keeps stderr in $tmp/err
  rc=0
  "$@" >"$tmp/stdout" 2>"$tmp/err" || rc=$?
}

# Fixture: a release zip with a stub exe and a license.
v=1.2.3
base="townjump-$v-x86_64-pc-windows-msvc"
mkdir -p "$tmp/fx/$base" "$tmp/fx-noexe/$base"
printf 'MZ stub executable\n' >"$tmp/fx/$base/townjump.exe"
printf 'Test license text\n' >"$tmp/fx/$base/LICENSE"
printf 'Test license text\n' >"$tmp/fx-noexe/$base/LICENSE"
(cd "$tmp/fx" && zip -qr "$tmp/$base.zip" "$base")
mkdir "$tmp/noexe"
(cd "$tmp/fx-noexe" && zip -qr "$tmp/noexe/$base.zip" "$base")

# 1. zip -> installer
run "$script" "$tmp/$base.zip" --out "$tmp/out"
expect_rc "zip builds an installer" 0 "$rc"
exe="$tmp/out/townjump-setup-$v.exe"
if [[ -f "$exe" ]]; then ok "output is townjump-setup-$v.exe"; else nok "output is townjump-setup-$v.exe"; fi
if grep -q "^installer: $exe\$" "$tmp/stdout"; then ok "prints installer path"; else nok "prints installer path"; fi
if [[ "$(head -c 2 "$exe")" == "MZ" ]]; then ok "starts with MZ"; else nok "starts with MZ"; fi
listing="$("$sz" l "$exe" 2>&1 || true)"
if grep -q 'Type = Nsis' <<<"$listing"; then ok "7z: Type = Nsis"; else nok "7z: Type = Nsis"; fi
for f in townjump.exe LICENSE.txt path-entry.ps1; do
  if grep -q "$f" <<<"$listing"; then ok "7z lists $f"; else nok "7z lists $f"; fi
done

# 2. bare exe
run "$script" "$tmp/fx/$base/townjump.exe" --out "$tmp/out-bare"
expect_rc "bare exe without --version" 2 "$rc"
run "$script" "$tmp/fx/$base/townjump.exe" --version 2.0.1 --out "$tmp/out-bare"
expect_rc "bare exe with --version builds" 0 "$rc"
if [[ -f "$tmp/out-bare/townjump-setup-2.0.1.exe" ]]; then ok "bare exe output named by --version"; else nok "bare exe output named by --version"; fi
run "$script" "$tmp/fx/$base/townjump.exe" --version 2.0 --out "$tmp/out-badv"
expect_rc "bad --version" 2 "$rc"

# 3. usage and bad input
run "$script" "$tmp/missing.zip" --out "$tmp/out-missing"
expect_rc "missing input" 2 "$rc"
run "$script"
expect_rc "no arguments" 2 "$rc"
run "$script" "$tmp/$base.zip" --bogus
expect_rc "unknown option" 2 "$rc"
run "$script" "$tmp/$base.zip" --out
expect_rc "--out without value" 2 "$rc"
run "$script" "$tmp/$base.zip" "$tmp/$base.zip"
expect_rc "two inputs" 2 "$rc"
cp "$tmp/$base.zip" "$tmp/townjump-1.2-x86_64-pc-windows-msvc.zip"
run "$script" "$tmp/townjump-1.2-x86_64-pc-windows-msvc.zip" --out "$tmp/out-v12"
expect_rc "zip name with version 1.2" 2 "$rc"
cp "$tmp/$base.zip" "$tmp/other.zip"
run "$script" "$tmp/other.zip" --out "$tmp/out-other"
expect_rc "zip with an unexpected name" 2 "$rc"
run "$script" "$tmp/noexe/$base.zip" --out "$tmp/out-noexe"
expect_rc "zip without townjump.exe" 1 "$rc"
leftover=""
for d in out-badv out-missing out-v12 out-other out-noexe; do
  [[ ! -e "$tmp/$d" ]] || leftover="$leftover $d"
done
if [[ -z "$leftover" ]]; then ok "failures leave no output"; else nok "failures leave output:$leftover"; fi

# 4. makensis out of PATH
mkdir "$tmp/bin"
for t in bash env unzip mktemp find head mv rm mkdir dirname basename cat grep; do
  p="$(command -v "$t" || true)"
  [[ -z "$p" ]] || ln -s "$p" "$tmp/bin/$t"
done
rc=0
PATH="$tmp/bin" "$(command -v bash)" "$script" "$tmp/$base.zip" --out "$tmp/out-nomk" >"$tmp/stdout" 2>"$tmp/err" || rc=$?
expect_rc "makensis missing" 1 "$rc"
if grep -q 'makensis' "$tmp/err"; then ok "missing makensis names the tool"; else nok "missing makensis names the tool"; fi
if [[ ! -e "$tmp/out-nomk" ]]; then ok "missing makensis leaves no output"; else nok "missing makensis leaves no output"; fi

# 5. makensis flags and failure (fake makensis)
mkdir "$tmp/fake"
cat >"$tmp/fake/makensis" <<'FAKE'
#!/usr/bin/env bash
printf '%s\n' "$@" >"$FAKE_ARGV"
[[ "${FAKE_FAIL:-0}" == 1 ]] && exit 1
for a in "$@"; do
  case "$a" in -DOUTFILE=*) printf 'MZ' >"${a#-DOUTFILE=}" ;; esac
done
exit 0
FAKE
chmod +x "$tmp/fake/makensis"
export FAKE_ARGV="$tmp/argv"
rc=0
PATH="$tmp/fake:$PATH" "$script" "$tmp/$base.zip" --out "$tmp/out-fake" >"$tmp/stdout" 2>"$tmp/err" || rc=$?
expect_rc "fake makensis succeeds" 0 "$rc"
if grep -qx -e '-WX' "$tmp/argv" && grep -qx -e '-V2' "$tmp/argv" && grep -qx -e "-DVERSION=$v" "$tmp/argv"; then
  ok "makensis gets -WX -V2 -DVERSION"; else nok "makensis gets -WX -V2 -DVERSION"; fi
if grep -q -e '-DLICENSE=.*LICENSE$' "$tmp/argv" && grep -q 'installer\.nsi$' "$tmp/argv"; then
  ok "makensis gets license and installer.nsi"; else nok "makensis gets license and installer.nsi"; fi
rc=0
FAKE_FAIL=1 PATH="$tmp/fake:$PATH" "$script" "$tmp/$base.zip" --out "$tmp/out-fail" >"$tmp/stdout" 2>"$tmp/err" || rc=$?
expect_rc "makensis failure" 1 "$rc"
if [[ ! -e "$tmp/out-fail" ]]; then ok "makensis failure leaves no output"; else nok "makensis failure leaves no output"; fi

echo "1..$n"
[[ $failed -eq 0 ]]
