#!/usr/bin/env bash
# Run brew style and brew audit --strict on the formula in a throw-away local tap.
# With --install <dir> <X.Y.Z>: also bump a copy against <dir>/SHA256SUMS (archives read from <dir> over file://),
# then brew install, brew test and brew uninstall it. The tap is always removed on exit.
set -euo pipefail

usage() {
  echo "Usage: brew-check.sh [--install <release-dir> <X.Y.Z>]" >&2
  exit 2
}

install_dir=""
version=""
case "$#" in
  0) ;;
  3)
    [ "$1" = --install ] || usage
    install_dir="$2"
    version="$3"
    ;;
  *) usage ;;
esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
formula="$here/../../packaging/homebrew/townjump.rb"
[ -f "$formula" ] || {
  echo "brew-check.sh: formula not found: $formula" >&2
  exit 2
}
if [ -n "$install_dir" ]; then
  [ -f "$install_dir/SHA256SUMS" ] || {
    echo "brew-check.sh: $install_dir/SHA256SUMS not found" >&2
    exit 2
  }
  install_dir="$(cd "$install_dir" && pwd)"
fi
command -v brew > /dev/null || {
  echo "brew-check.sh: brew not found" >&2
  exit 1
}

export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
tap=local/townjump-check
work="$(mktemp -d)"
installed=""

cleanup() {
  local rc=$?
  trap - EXIT
  if [ -n "$installed" ]; then brew uninstall --formula "$tap/townjump" > /dev/null 2>&1 || true; fi
  if brew tap | grep -qx "$tap"; then
    if brew commands 2> /dev/null | grep -qx untrust; then brew untrust --tap "$tap" > /dev/null 2>&1 || true; fi
    brew untap "$tap" > /dev/null 2>&1 || true
  fi
  rm -rf "$work"
  exit "$rc"
}
trap cleanup EXIT

# A tap left over from an interrupted run is removed first.
if brew tap | grep -qx "$tap"; then brew untap "$tap" > /dev/null 2>&1 || true; fi

brew tap-new --no-git "$tap" > /dev/null
if brew commands 2> /dev/null | grep -qx trust; then brew trust "$tap"; fi
tap_dir="$(brew --repository "$tap")"
mkdir -p "$tap_dir/Formula"
cp "$formula" "$tap_dir/Formula/townjump.rb"

echo "# brew style"
brew style --formula "$tap/townjump"
echo "# brew audit --strict"
brew audit --strict --formula "$tap/townjump"

if [ -n "$install_dir" ]; then
  # brew reads the version from the url, so the release directory is exposed under a v<version> name.
  ln -s "$install_dir" "$work/v$version"
  BUMP_FORMULA="$tap_dir/Formula/townjump.rb" "$here/bump-formula.sh" "$version" \
    --sums "$install_dir/SHA256SUMS" --base-url "file://$work/v$version"
  echo "# brew install"
  installed=1
  brew install --formula "$tap/townjump"
  echo "# brew test"
  brew test "$tap/townjump"
  echo "# brew uninstall"
  brew uninstall --formula "$tap/townjump"
  installed=""
fi
echo "brew-check: ok"
