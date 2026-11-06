#!/usr/bin/env bash
# Tests for .gitattributes: the landing page sources stay out of the release source archive. No network access.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fails=0
pass() { echo "ok - $1"; }
fail() { fails=$((fails + 1)); echo "not ok - $1"; }

# site_dir_is_export_ignored: a temporary repository with the same attributes file.
mkdir -p "$tmp/r/site" "$tmp/r/crates"
cp "$root/.gitattributes" "$tmp/r/.gitattributes"
echo '<html></html>' > "$tmp/r/site/index.html"
echo x > "$tmp/r/crates/x"
git -C "$tmp/r" init -q
git -C "$tmp/r" add -A
git -C "$tmp/r" -c user.name=t -c user.email=t@example.invalid commit -q -m t
git -C "$tmp/r" archive --format=tar --prefix=p/ -o "$tmp/a.tar" HEAD
list="$(tar -tf "$tmp/a.tar")"
if ! printf '%s\n' "$list" | grep -q '^p/site/' && printf '%s\n' "$list" | grep -q '^p/crates/x$'; then
  pass "site_dir_is_export_ignored: archive"
else
  fail "site_dir_is_export_ignored: archive"
fi
if [ "$(git -C "$root" check-attr export-ignore -- site/index.html | sed 's/.*: //')" = set ]; then
  pass "site_dir_is_export_ignored: attribute"
else
  fail "site_dir_is_export_ignored: attribute"
fi

# release_workflow_uses_git_archive
if grep -q 'git archive' "$root/.github/workflows/release.yml"; then
  pass "release_workflow_uses_git_archive"
else
  fail "release_workflow_uses_git_archive"
fi

[ "$fails" -eq 0 ] || { echo "# $fails failed"; exit 1; }
echo "# all passed"
