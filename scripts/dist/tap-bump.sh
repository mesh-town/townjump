#!/usr/bin/env bash
# Copy the townjump formula into the Homebrew tap, bump it to a release and push one commit.
# Usage: tap-bump.sh <X.Y.Z> [bump-formula.sh arguments...]
# Env: TAP_TOKEN (empty: skip), TAP_REPO_URL (default: the tap on GitHub, authenticated with TAP_TOKEN),
#      TAP_SOURCE_FORMULA (default: packaging/homebrew/townjump.rb of this repository).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$#" -ge 1 ] || {
  echo "Usage: tap-bump.sh <X.Y.Z> [bump-formula.sh arguments...]" >&2
  exit 2
}
version="$1"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "tap-bump.sh: bad version '$version' (want X.Y.Z)" >&2
  exit 2
}

if [ -z "${TAP_TOKEN:-}" ]; then
  echo "::notice::TAP_TOKEN is not set; the Homebrew tap is not updated"
  exit 0
fi

source_formula="${TAP_SOURCE_FORMULA:-$here/../../packaging/homebrew/townjump.rb}"
[ -f "$source_formula" ] || {
  echo "tap-bump.sh: formula not found: $source_formula" >&2
  exit 2
}
repo_url="${TAP_REPO_URL:-https://x-access-token:${TAP_TOKEN}@github.com/mesh-town/homebrew-macos.git}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git clone --quiet "$repo_url" "$work/tap" || {
  echo "tap-bump.sh: cannot clone the tap" >&2
  exit 1
}

mkdir -p "$work/tap/Formula"
cp "$source_formula" "$work/tap/Formula/townjump.rb"
BUMP_FORMULA="$work/tap/Formula/townjump.rb" "$here/bump-formula.sh" "$@" || {
  echo "tap-bump.sh: formula bump failed; nothing pushed" >&2
  exit 1
}

cd "$work/tap"
git add Formula/townjump.rb
if git diff --cached --quiet; then
  echo "tap is up to date"
  exit 0
fi

d="$(date -u +%F)T12:00:00+0000"
export TZ=UTC
export GIT_AUTHOR_NAME='Mesh Town' GIT_AUTHOR_EMAIL='340663027+mesh-town@users.noreply.github.com'
export GIT_COMMITTER_NAME='Mesh Town' GIT_COMMITTER_EMAIL='340663027+mesh-town@users.noreply.github.com'
export GIT_AUTHOR_DATE="$d" GIT_COMMITTER_DATE="$d"
git commit --quiet -m "townjump $version"
git push --quiet origin HEAD || {
  echo "tap-bump.sh: push failed" >&2
  exit 1
}
echo "tap updated to townjump $version"
