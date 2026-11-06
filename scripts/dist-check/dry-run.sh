#!/usr/bin/env bash
# dry-run.sh [all|deb|dist|openwrt|keenetic] [--assets DIR] [--allow-partial]
# Builds, checks and "publishes" the Linux package feeds on this machine; everything lives in target/dist-dry/ and
# nothing leaves it (publishing goes to the local bare repositories target/dist-dry/{dist,openwrt,keenetic}.git). Archives come from
# --assets or from local_assets.sh (needs CARGO_TARGET_DIR; the caller holds the machine lock). Throwaway keys only.
# Checks per mode: builder, *_check.py and leak_check.py, a second build must give the same sha256 for .deb/.ipk/Packages,
# dist: the direct-download tree of a fixture release (fake-release.sh, no cargo) is built, checked and published to the dist
# repository keeping linux/debian; with deb done before, both publishing orders (dist then deb, deb then dist) must keep both.
# deb: a third build with pkg-release 2 must leave both -1 and -2 in the published pool, then container.sh debian.
# Publishing goes to three local bare repositories (dist, openwrt, keenetic) that mirror the three real ones.
# Exit codes: 0 done, 1 failure, 2 bad arguments, 3 a part was skipped (unless --allow-partial); "skipped: <why>" is printed.
set -euo pipefail

usage() { echo "usage: dry-run.sh [all|deb|dist|openwrt|keenetic] [--assets DIR] [--allow-partial]" >&2; exit 2; }
mode="all" assets="" partial_ok=0 seen_mode=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    all | deb | dist | openwrt | keenetic) [ "$seen_mode" = 0 ] || usage; mode="$1"; seen_mode=1 ;;
    --assets) [ "$#" -ge 2 ] || usage; assets="$2"; shift ;;
    --allow-partial) partial_ok=1 ;;
    *) usage ;;
  esac
  shift
done
if [ -n "$assets" ] && [ ! -d "$assets" ]; then
  echo "dry-run.sh: assets directory '$assets' does not exist" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
dry="$root/target/dist-dry"
V="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
export PYTHONDONTWRITEBYTECODE=1
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct HEAD)}"
# shellcheck source=/dev/null
. "$root/packaging/openwrt/common/site.env"
partial=0
gpghome=""

skip() { echo "skipped: $1"; partial=1; }
fail() { echo "dry-run.sh: $1" >&2; exit 1; }
cleanup() {
  if [ -n "$gpghome" ]; then gpgconf --homedir "$gpghome" --kill gpg-agent > /dev/null 2>&1 || true; rm -rf "$gpghome"; fi
}
trap cleanup EXIT

# sums <dir> -> sha256 of every .deb/.ipk/Packages below it, relative paths, sorted
sums() {
  (cd "$1" && find . -type f \( -name '*.deb' -o -name '*.ipk' -o -name Packages \) -print | LC_ALL=C sort | xargs shasum -a 256)
}

same_bytes() { # <out-a> <out-b>
  local a b
  a="$(sums "$1")" b="$(sums "$2")"
  [ -n "$a" ] || fail "no .deb/.ipk/Packages files under $1"
  [ "$a" = "$b" ] || { printf '%s\n--\n%s\n' "$a" "$b" >&2; fail "second build differs from the first ($1 vs $2)"; }
  echo "reproducible: $(printf '%s\n' "$a" | wc -l | tr -d ' ') files identical"
  printf '%s\n' "$a"
}

make_assets() {
  local triples="$1"
  if [ -z "$assets" ]; then
    assets="$dry/assets"
    # shellcheck disable=SC2086
    "$here/local_assets.sh" "$assets" "$V" $triples
  fi
}

make_gpg_keys() {
  mkdir -p "$dry/keys"
  # A short path: gpg-agent sockets do not fit into the long worktree path.
  gpghome="$(mktemp -d /tmp/tjgpg.XXXXXX)"
  gpg --homedir "$gpghome" --batch --pinentry-mode loopback --passphrase '' \
    --quick-generate-key 'Townjump Dry Run <dry-run@example.invalid>' ed25519 sign never 2> /dev/null
  gpg --homedir "$gpghome" --batch --armor --export-secret-keys > "$dry/keys/apt-secret.asc"
  gpg --homedir "$gpghome" --batch --export > "$dry/keys/apt-public.gpg"
}

new_pages() { # <repo>
  [ -d "$dry/$1.git" ] || git init -q --bare -b "$PAGES_BRANCH" "$dry/$1.git"
}
publish() { # <repo:dist|openwrt|keenetic> <subpath|-> <src> [--update CMD]
  local repo="$1" sub="$2" src="$3"
  shift 3
  new_pages "$repo"
  "$root/packaging/openwrt/common/publish_pages.sh" "$sub" "$src" "file://$dry/$repo.git" "$PAGES_BRANCH" "$V" "$@"
}

# The apt source shipped inside the amd64 package must point at the address the dist repository serves it from.
check_apt_address() { # <debs dir>
  local want uris
  want="$(site_url DIST_PAGES_URL)/linux/debian"
  uris="$(python3 -I - "$here" "$1/townjump_${V}-1_amd64.deb" <<'PY'
import sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import deb_check
members = dict(deb_check.parse_ar(Path(sys.argv[2]).read_bytes()))
for m, body in deb_check.read_tar(members["data.tar.gz"], "data.tar.gz"):
    if m.name.endswith("etc/apt/sources.list.d/townjump.sources"):
        for line in body.decode().splitlines():
            if line.startswith("URIs:"):
                print(line.split(":", 1)[1].strip())
PY
)"
  [ "$uris" = "$want" ] || fail "townjump.sources has URIs '$uris', want '$want'"
  echo "apt source address: $uris"
}

run_deb() {
  echo "== deb"
  make_gpg_keys
  export TOWNJUMP_APT_KEY="$dry/keys/apt-secret.asc" TOWNJUMP_APT_PUBKEY="$dry/keys/apt-public.gpg"
  local last
  last="$("$root/scripts/dist/deb-build.sh" "$assets" "$dry/out-deb" "$V" 1 | tee /dev/stderr | tail -n 1)"
  [ "$last" = "signed=false" ] || fail "throwaway keys must give signed=false, got '$last'"
  "$root/scripts/dist/deb-build.sh" "$assets" "$dry/out-deb-b" "$V" 1 > /dev/null
  same_bytes "$dry/out-deb" "$dry/out-deb-b"
  python3 "$here/leak_check.py" "$dry/out-deb"
  python3 "$here/apt_check.py" "$dry/out-deb/apt" --pubkey "$TOWNJUMP_APT_PUBKEY"
  check_apt_address "$dry/out-deb/debs"
  publish dist linux/debian "$dry/out-deb/debs" --update "$root/scripts/dist/deb-update.sh"
  "$root/scripts/dist/deb-build.sh" "$assets" "$dry/out-deb-n2" "$V" 2 > /dev/null
  publish dist linux/debian "$dry/out-deb-n2/debs" --update "$root/scripts/dist/deb-update.sh"
  pool="$(git -C "$dry/dist.git" ls-tree -r --name-only "$PAGES_BRANCH" -- linux/debian/pool)"
  for rel in "$V-1" "$V-2"; do
    printf '%s\n' "$pool" | grep -q "townjump_${rel}_amd64.deb" || fail "pool lacks townjump_${rel}_amd64.deb"
  done
  echo "pool keeps $V-1 and $V-2"
  deb_done=1
  container_gate debian
}

dist_published=0 deb_done=0

publish_dist_tree() { # <repo>
  ( export PAGES_KEEP=linux/debian; publish "$1" - "$dry/out-dist" )
}
publish_debs() { # <repo>
  publish "$1" linux/debian "$dry/out-deb/debs" --update "$root/scripts/dist/deb-update.sh"
}

# Both files of the dist repository must survive in the repository whatever was published last.
dist_has_both() { # <repo>
  local repo="$1" f
  for f in macos/townjump-macos-aarch64.tar.gz linux/townjump-linux-x86_64.tar.gz windows/townjump.exe android/townjump-termux-aarch64.tar.gz \
    linux/debian/dists/stable/InRelease; do
    git -C "$dry/$repo.git" cat-file -e "$PAGES_BRANCH:$f" || fail "$repo: $f is missing after publishing both"
  done
}

run_dist() {
  echo "== dist"
  "$here/../dist/fake-release.sh" "$V" "$dry/fake-release"
  "$here/../dist/dist-tree.sh" "$dry/fake-release" "$dry/out-dist" "$V"
  "$here/../dist/dist-tree.sh" "$dry/fake-release" "$dry/out-dist-b" "$V" > /dev/null
  diff -r "$dry/out-dist" "$dry/out-dist-b" > /dev/null || fail "second dist tree differs from the first"
  python3 "$here/dist_tree_check.py" "$dry/out-dist"
  publish_dist_tree dist
  dist_published=1
  if [ "$deb_done" = 1 ]; then
    publish_dist_tree dist-ab
    publish_debs dist-ab
    dist_has_both dist-ab
    publish_debs dist-ba
    publish_dist_tree dist-ba
    dist_has_both dist-ba
    echo "dist then deb and deb then dist keep both"
  fi
}

run_keenetic() {
  echo "== keenetic"
  "$root/scripts/dist/keenetic-build.sh" "$assets" "$dry/out-keenetic" "$V" 1 --strict
  "$root/scripts/dist/keenetic-build.sh" "$assets" "$dry/out-keenetic-b" "$V" 1 --strict > /dev/null
  same_bytes "$dry/out-keenetic" "$dry/out-keenetic-b"
  python3 "$here/leak_check.py" "$dry/out-keenetic/pages"
  publish keenetic - "$dry/out-keenetic/pages"
}

# The feed builder needs usign, apk and root (fakeroot). Without them on the host (macOS) the build runs in a Linux container.
openwrt_on_host() { command -v usign > /dev/null 2>&1 && command -v apk > /dev/null 2>&1 && command -v fakeroot > /dev/null 2>&1; }

build_openwrt() {
  local builder="$root/scripts/dist/openwrt-build.sh" rc=0
  if openwrt_on_host; then
    fakeroot "$builder" "$assets" "$dry/out-openwrt" "$V" 1 --strict --ephemeral-keys
    fakeroot "$builder" "$assets" "$dry/out-openwrt-b" "$V" 1 --strict --ephemeral-keys > /dev/null
    return 0
  fi
  [ "${DRY_SKIP_CONTAINER:-0}" != 1 ] || { skip "no usign/apk on the host and the container is disabled"; return 1; }
  V="$V" "$here/container.sh" openwrt-build "$dry" || rc=$?
  [ "$rc" = 0 ] && return 0
  [ "$rc" = 3 ] || fail "container build of the OpenWrt feed failed ($rc)"
  partial=1
  return 1
}

run_openwrt() {
  echo "== openwrt"
  [ -x "$root/scripts/dist/openwrt-build.sh" ] || { skip "scripts/dist/openwrt-build.sh is not present"; return 0; }
  build_openwrt || return 0
  same_bytes "$dry/out-openwrt/pages" "$dry/out-openwrt-b/pages"
  python3 "$here/leak_check.py" "$dry/out-openwrt/pages"
  publish openwrt - "$dry/out-openwrt/pages"
  container_gate alpine
}

container_gate() {
  local rc=0
  if [ "${DRY_SKIP_CONTAINER:-0}" = 1 ]; then skip "container gate disabled by DRY_SKIP_CONTAINER"; return 0; fi
  "$here/container.sh" "$1" "$dry" || rc=$?
  case "$rc" in 0) ;; 3) partial=1 ;; *) fail "container gate $1 failed ($rc)" ;;
  esac
}

site_url() { sed -n "s/^$1=//p" "$root/packaging/openwrt/common/site.env"; }

check_repo() { # <repo> <expected files, space separated>
  local repo="$1" want="$2" n f
  n="$(git -C "$dry/$repo.git" rev-list --count "$PAGES_BRANCH")"
  [ "$n" = 1 ] || fail "$repo: $PAGES_BRANCH has $n commits, expected 1"
  [ -z "$(git -C "$dry/$repo.git" log -1 --format=%P "$PAGES_BRANCH")" ] || fail "$repo: the Pages commit has a parent"
  git -C "$dry/$repo.git" log -1 --format='%an <%ae> %ad' --date=iso "$PAGES_BRANCH" | grep -q 'Mesh Town .* +0000$' \
    || fail "$repo: the Pages commit is not authored by Mesh Town at +0000"
  for f in $want; do
    git -C "$dry/$repo.git" cat-file -e "$PAGES_BRANCH:$f" || fail "$repo: $f is missing from the published tree"
  done
  echo "$repo tree: $(git -C "$dry/$repo.git" ls-tree --name-only "$PAGES_BRANCH" | tr '\n' ' ')"
}

check_install_address() { # <repo> <site.env key>
  local want
  want="$(site_url "$2")"
  [ -n "$want" ] || fail "$2 is not set in site.env"
  git -C "$dry/$1.git" show "$PAGES_BRANCH:install.sh" | grep -qF "$want" || fail "$1: install.sh does not carry $want"
  git -C "$dry/$1.git" show "$PAGES_BRANCH:README.md" | grep -qF "$want" || fail "$1: README.md does not carry $want"
}

final_checks() {
  if [ -d "$dry/dist.git" ]; then
    local want=".nojekyll"
    [ "$deb_done" != 1 ] || want="$want linux/debian/dists/stable/InRelease"
    [ "$dist_published" != 1 ] || want="$want SHA256SUMS version.txt README.md macos/townjump-macos-x86_64.tar.gz linux/townjump-linux-mips.tar.gz android/townjump-termux-aarch64.tar.gz windows/townjump.exe windows/townjump-setup.exe"
    check_repo dist "$want"
  fi
  if [ -d "$dry/openwrt.git" ]; then
    check_repo openwrt "install.sh .nojekyll README.md"
    check_install_address openwrt OPENWRT_PAGES_URL
  fi
  if [ -d "$dry/keenetic.git" ]; then
    check_repo keenetic "install.sh .nojekyll README.md"
    check_install_address keenetic KEENETIC_PAGES_URL
  fi
  return 0
}

before="$(git -C "$root" status --porcelain)"
mkdir -p "$dry"
[ "$mode" != all ] || rm -rf "$dry/pages.git" "$dry/dist.git" "$dry/dist-ab.git" "$dry/dist-ba.git" "$dry/openwrt.git" "$dry/keenetic.git"
case "$mode" in
  dist) ;;
  deb) make_assets "x86_64-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf" ;;
  keenetic) make_assets "aarch64-unknown-linux-musl mipsel-unknown-linux-musl mips-unknown-linux-musl" ;;
  openwrt) make_assets "x86_64-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf armv7-unknown-linux-musleabi mipsel-unknown-linux-musl mips-unknown-linux-musl" ;;
  *) make_assets "" ;;
esac
case "$mode" in
  deb) run_deb ;;
  dist) run_dist ;;
  keenetic) run_keenetic ;;
  openwrt) run_openwrt ;;
  all) run_deb; run_dist; run_openwrt; run_keenetic ;;
esac
final_checks
[ "$(git -C "$root" status --porcelain)" = "$before" ] || fail "the repository tree changed during the run"
if [ "$partial" = 1 ] && [ "$partial_ok" = 0 ]; then
  echo "dry-run.sh: finished with skipped parts"
  exit 3
fi
echo "dry-run.sh: $mode done"
