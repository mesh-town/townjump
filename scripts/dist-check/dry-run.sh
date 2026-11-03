#!/usr/bin/env bash
# dry-run.sh [all|deb|openwrt|keenetic] [--assets DIR] [--allow-partial]
# Builds, checks and "publishes" the Linux package feeds on this machine; everything lives in target/dist-dry/ and
# nothing leaves it (publishing goes to the local bare repository target/dist-dry/pages.git). Archives come from
# --assets or from local_assets.sh (needs CARGO_TARGET_DIR; the caller holds the machine lock). Throwaway keys only.
# Checks per mode: builder, *_check.py and leak_check.py, a second build must give the same sha256 for .deb/.ipk/Packages,
# deb: a third build with pkg-release 2 must leave both -1 and -2 in the published pool, then container.sh debian.
# Exit codes: 0 done, 1 failure, 2 bad arguments, 3 a part was skipped (unless --allow-partial); "skipped: <why>" is printed.
set -euo pipefail

usage() { echo "usage: dry-run.sh [all|deb|openwrt|keenetic] [--assets DIR] [--allow-partial]" >&2; exit 2; }
mode="all" assets="" partial_ok=0 seen_mode=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    all | deb | openwrt | keenetic) [ "$seen_mode" = 0 ] || usage; mode="$1"; seen_mode=1 ;;
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

new_pages() { [ -d "$dry/pages.git" ] || git init -q --bare "$dry/pages.git"; }
publish() { # <subdir> <src> [--update CMD]
  local sub="$1" src="$2"
  shift 2
  "$root/packaging/openwrt/common/publish_pages.sh" "$sub" "$src" "file://$dry/pages.git" "$PAGES_BRANCH" "$V" "$@"
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
  new_pages
  publish apt "$dry/out-deb/debs" --update "$root/scripts/dist/deb-update.sh"
  "$root/scripts/dist/deb-build.sh" "$assets" "$dry/out-deb-n2" "$V" 2 > /dev/null
  publish apt "$dry/out-deb-n2/debs" --update "$root/scripts/dist/deb-update.sh"
  pool="$(git -C "$dry/pages.git" ls-tree -r --name-only "$PAGES_BRANCH" -- apt/pool)"
  for rel in "$V-1" "$V-2"; do
    printf '%s\n' "$pool" | grep -q "townjump_${rel}_amd64.deb" || fail "pool lacks townjump_${rel}_amd64.deb"
  done
  echo "pool keeps $V-1 and $V-2"
  container_gate debian
}

run_keenetic() {
  echo "== keenetic"
  "$root/scripts/dist/keenetic-build.sh" "$assets" "$dry/out-keenetic" "$V" 1 --strict
  "$root/scripts/dist/keenetic-build.sh" "$assets" "$dry/out-keenetic-b" "$V" 1 --strict > /dev/null
  same_bytes "$dry/out-keenetic" "$dry/out-keenetic-b"
  python3 "$here/leak_check.py" "$dry/out-keenetic/pages"
  new_pages
  publish keenetic "$dry/out-keenetic/pages"
}

run_openwrt() {
  echo "== openwrt (not run until the OpenWrt builder is merged)"
  local builder="$root/scripts/dist/openwrt-build.sh" only=()
  [ -x "$builder" ] || { skip "scripts/dist/openwrt-build.sh is not present"; return 0; }
  if ! command -v apk > /dev/null 2>&1 && ! command -v container > /dev/null 2>&1; then
    skip "no apk and no container: apk feed not built"
    only=(--only ipk)
  fi
  "$builder" "$assets" "$dry/out-openwrt" "$V" 1 --strict "${only[@]}" --ephemeral-keys
  "$builder" "$assets" "$dry/out-openwrt-b" "$V" 1 --strict "${only[@]}" --ephemeral-keys > /dev/null
  same_bytes "$dry/out-openwrt/pages" "$dry/out-openwrt-b/pages"
  python3 "$here/leak_check.py" "$dry/out-openwrt/pages"
  new_pages
  publish openwrt "$dry/out-openwrt/pages"
}

container_gate() {
  local rc=0
  if [ "${DRY_SKIP_CONTAINER:-0}" = 1 ]; then skip "container gate disabled by DRY_SKIP_CONTAINER"; return 0; fi
  "$here/container.sh" "$1" "$dry" || rc=$?
  case "$rc" in 0) ;; 3) partial=1 ;; *) fail "container gate $1 failed ($rc)" ;;
  esac
}

final_checks() {
  [ -d "$dry/pages.git" ] || return 0
  local n tree
  n="$(git -C "$dry/pages.git" rev-list --count "$PAGES_BRANCH")"
  [ "$n" = 1 ] || fail "$PAGES_BRANCH has $n commits, expected 1"
  [ -z "$(git -C "$dry/pages.git" log -1 --format=%P "$PAGES_BRANCH")" ] || fail "the Pages commit has a parent"
  git -C "$dry/pages.git" log -1 --format='%an <%ae> %ad' --date=iso "$PAGES_BRANCH" | grep -q 'Mesh Town .* +0000$' \
    || fail "the Pages commit is not authored by Mesh Town at +0000"
  tree="$(git -C "$dry/pages.git" ls-tree --name-only "$PAGES_BRANCH" | tr '\n' ' ')"
  echo "pages tree: $tree"
}

before="$(git -C "$root" status --porcelain)"
mkdir -p "$dry"
[ "$mode" != all ] || rm -rf "$dry/pages.git"
case "$mode" in
  deb) make_assets "x86_64-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf" ;;
  keenetic) make_assets "aarch64-unknown-linux-musl mipsel-unknown-linux-musl mips-unknown-linux-musl" ;;
  *) make_assets "" ;;
esac
case "$mode" in
  deb) run_deb ;;
  keenetic) run_keenetic ;;
  openwrt) run_openwrt ;;
  all) run_deb; run_openwrt; run_keenetic ;;
esac
final_checks
[ "$(git -C "$root" status --porcelain)" = "$before" ] || fail "the repository tree changed during the run"
if [ "$partial" = 1 ] && [ "$partial_ok" = 0 ]; then
  echo "dry-run.sh: finished with skipped parts"
  exit 3
fi
echo "dry-run.sh: $mode done"
