#!/usr/bin/env bash
# TAP tests for dist-tree.sh on a fixture release (fake-release.sh). No network.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tree="$here/dist-tree.sh"
fake="$here/fake-release.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/disttree.XXXXXX")"
trap 'rm -rf "$work"' EXIT
V=1.2.3

n=0
fail=0
ok() { n=$((n + 1)); echo "ok $n - $1"; }
nok() { n=$((n + 1)); fail=$((fail + 1)); echo "not ok $n - $1"; }
check() { if "$@"; then ok "$name"; else nok "$name"; fi; }
rc() { set +e; "$@" >"$work/out" 2>"$work/err"; RC=$?; set -e; }
fresh() { rm -rf "$work/rel" "$work/tree"; "$fake" "$V" "$work/rel" >/dev/null 2>&1; }

expected_files="macos/townjump-macos-aarch64.tar.gz macos/townjump-macos-x86_64.tar.gz
linux/townjump-linux-x86_64.tar.gz linux/townjump-linux-aarch64.tar.gz linux/townjump-linux-armv7hf.tar.gz
linux/townjump-linux-armv7sf.tar.gz linux/townjump-linux-mipsel.tar.gz linux/townjump-linux-mips.tar.gz
android/townjump-termux-aarch64.tar.gz windows/townjump.exe windows/townjump-setup.exe"

name=tree_layout_matches_spec
fresh
rc "$tree" "$work/rel" "$work/tree" "$V"
good=1
[ "$RC" = 0 ] || good=0
for f in $expected_files SHA256SUMS version.txt README.md .nojekyll; do [ -f "$work/tree/$f" ] || { good=0; echo "# missing $f"; }; done
[ "$(cd "$work/tree" && find macos linux android windows -type f | wc -l | tr -d ' ')" = 11 ] || good=0
cmp -s "$work/rel/townjump-$V-armv7-unknown-linux-musleabihf.tar.gz" "$work/tree/linux/townjump-linux-armv7hf.tar.gz" || good=0
cmp -s "$work/rel/townjump-$V-armv7-unknown-linux-musleabi.tar.gz" "$work/tree/linux/townjump-linux-armv7sf.tar.gz" || good=0
cmp -s "$work/rel/townjump-$V-aarch64-linux-android.tar.gz" "$work/tree/android/townjump-termux-aarch64.tar.gz" || good=0
cmp -s "$work/rel/townjump-setup-$V.exe" "$work/tree/windows/townjump-setup.exe" || good=0
check test "$good" = 1

name=bad_checksum_fails
fresh
echo extra >>"$work/rel/townjump-$V-aarch64-apple-darwin.tar.gz"
rc "$tree" "$work/rel" "$work/tree" "$V"
check test "$RC" = 1

name=missing_archive_fails_naming_it
fresh
rm "$work/rel/townjump-$V-mipsel-unknown-linux-musl.tar.gz"
rc "$tree" "$work/rel" "$work/tree" "$V"
check bash -c "[ $RC = 1 ] && grep -q 'townjump-$V-mipsel-unknown-linux-musl.tar.gz' '$work/err'"

name=windows_exe_extracted_from_zip
fresh
rc "$tree" "$work/rel" "$work/tree" "$V"
check bash -c "[ \"\$(head -c 2 '$work/tree/windows/townjump.exe')\" = MZ ] && grep -q 'x86_64-pc-windows-msvc' '$work/tree/windows/townjump.exe'"

name=sums_cover_tree_files
listed="$(awk '{print $2}' "$work/tree/SHA256SUMS" | LC_ALL=C sort)"
found="$(cd "$work/tree" && find macos linux android windows -type f | LC_ALL=C sort)"
check bash -c "[ \"\$1\" = \"\$2\" ] && cd '$work/tree' && shasum -a 256 -c SHA256SUMS >/dev/null" _ "$listed" "$found"

name=version_txt_and_readme_have_version
check bash -c "[ \"\$(cat '$work/tree/version.txt')\" = $V ] && grep -q '$V' '$work/tree/README.md' && ! grep -q '@VERSION@' '$work/tree/README.md'"

name=no_versions_in_file_names
bad="$(cd "$work/tree" && find . -type f | grep -E '[0-9]+\.[0-9]+\.[0-9]+' || true)"
check test -z "$bad"

name=zip_without_exe_fails
fresh
z="townjump-$V-x86_64-pc-windows-msvc.zip"
mkdir "$work/zipsrc"
echo x >"$work/zipsrc/other.txt"
rm "$work/rel/$z"
(cd "$work/zipsrc" && zip -q "$work/rel/$z" other.txt)
rm -rf "$work/zipsrc"
sed "/$z/d" "$work/rel/SHA256SUMS" >"$work/sums.new"
printf '%s  %s\n' "$(shasum -a 256 "$work/rel/$z" | cut -d' ' -f1)" "$z" >>"$work/sums.new"
mv "$work/sums.new" "$work/rel/SHA256SUMS"
rc "$tree" "$work/rel" "$work/tree" "$V"
check bash -c "[ $RC = 1 ] && grep -q 'no townjump.exe' '$work/err'"

name=unknown_arg_exit_2
rc "$tree" "$work/rel" "$work/tree"
a=$RC
rc "$tree" "$work/rel" "$work/tree" "$V" extra
b=$RC
rc "$tree" "$work/rel" "$work/tree" 1.2
c=$RC
rc "$tree" "$work/nonexistent" "$work/tree" "$V"
check test "$a$b$c$RC" = 2222

name=debian_dir_not_created
check test ! -e "$work/tree/linux/debian"

echo "1..$n"
[[ "$fail" -eq 0 ]]
