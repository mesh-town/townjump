#!/usr/bin/env bash
# TAP tests for gen-keys.sh and install-public-keys.sh. Keys live in a temporary directory only.
# Without usign the usign-dependent tests are skipped (TOWNJUMP_PKG_REQUIRE_TOOLS=1 turns a skip into a failure).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
gen="$here/gen-keys.sh"
inst="$here/install-public-keys.sh"
work="$(mktemp -d /tmp/genkeys-test.XXXXXX)"
trap 'rm -rf "$work"' EXIT

n=0 fail=0
ok() { n=$((n + 1)); echo "ok $n - $1"; }
nok() { n=$((n + 1)); fail=$((fail + 1)); echo "not ok $n - $1"; }
skip() {
  if [ "${TOWNJUMP_PKG_REQUIRE_TOOLS:-}" = 1 ]; then nok "$1 (skipped: $2)"; else n=$((n + 1)); echo "ok $n - $1 # SKIP $2"; fi
}
rc() { set +e; "$@" >"$work/out" 2>"$work/err"; RC=$?; set -e; }
expect() { if [ "$2" = "$3" ]; then ok "$1"; else nok "$1 (rc $3, want $2)"; fi; }

usign_bin="${USIGN:-$(command -v usign || true)}"
if [ -z "$usign_bin" ] || [ ! -x "$usign_bin" ]; then
  echo "# usign not found: every test needing it is skipped"
  for t in generates_all_files modes_are_private public_key_verifies_signature_apt public_key_verifies_signature_openwrt \
    deploy_keys_are_distinct allows_ignored_dir_inside_repo install_public_keys_places_files install_rejects_mismatched_pair install_rejects_mismatched_pair_apt \
    install_rejects_mismatched_pair_apk force_regenerates_complete_set no_private_material_in_repo_after_install; do skip "$t" "usign is not installed"; done
  have_usign=false
else
  have_usign=true
  export USIGN="$usign_bin"
fi

marker="PRIVATE"" KEY"
keys="$work/keys"
if $have_usign; then
  rc "$gen" "$keys"
  expect "generates_all_files (exit)" 0 "$RC"
  missing=""
  for f in townjump-apt.sec.asc townjump.gpg townjump.asc fingerprints.txt townjump-apt.rev townjump-usign.sec townjump-usign.pub \
    townjump-apk.sec townjump-apk.pub deploy-dist deploy-dist.pub deploy-openwrt deploy-openwrt.pub deploy-keenetic deploy-keenetic.pub; do
    [ -s "$keys/$f" ] || missing="$missing $f"
  done
  if [ -z "$missing" ] && grep -q '^gpg-fingerprint: [0-9A-F]\{40\}$' "$keys/fingerprints.txt" \
    && grep -q 'APT_GPG_KEY' "$work/out" && grep -q 'townjump-dist' "$work/out" \
    && ! grep -q "$marker" "$work/out"; then ok "generates_all_files"; else nok "generates_all_files (missing:$missing)"; fi

  bad=""
  for f in "$keys"/*; do [ "$(stat -c %a "$f" 2>/dev/null || stat -f %Lp "$f")" = 600 ] || bad="$bad $f"; done
  [ "$(stat -c %a "$keys" 2>/dev/null || stat -f %Lp "$keys")" = 700 ] || bad="$bad dir"
  if [ -z "$bad" ]; then ok "modes_are_private"; else nok "modes_are_private ($bad)"; fi
fi

# A directory inside a repository is refused (and nothing is created there).
mkdir "$work/repo" && git -C "$work/repo" init -q
rc "$gen" "$work/repo/keys"
if [ "$RC" = 1 ] && [ ! -e "$work/repo/keys" ] && grep -q 'inside a git repository' "$work/err"; then ok "refuses_dir_inside_repo"; else nok "refuses_dir_inside_repo (rc $RC)"; fi

# A repository that ignores only some of the secret names is refused; one that ignores the whole key path is accepted.
mkdir "$work/repo2" && git -C "$work/repo2" init -q && printf 'keys/deploy-dist\n' >"$work/repo2/.gitignore"
rc "$gen" "$work/repo2/keys"
if [ "$RC" = 1 ] && [ ! -e "$work/repo2/keys" ] && grep -q 'does not ignore it' "$work/err"; then ok "refuses_partly_ignored_dir_inside_repo"; else nok "refuses_partly_ignored_dir_inside_repo (rc $RC)"; fi
if $have_usign; then
  mkdir "$work/repo3" && git -C "$work/repo3" init -q && printf 'keys/\n' >"$work/repo3/.gitignore"
  set +e; (cd "$work/repo3" && "$gen" keys >"$work/out" 2>"$work/err"); RC=$?; set -e
  if [ "$RC" = 0 ] && [ -s "$work/repo3/keys/deploy-dist" ] && [ "$(git -C "$work/repo3" status --porcelain)" = "?? .gitignore" ]; then ok "allows_ignored_dir_inside_repo"; else nok "allows_ignored_dir_inside_repo (rc $RC)"; fi
fi

mkdir "$work/full" && : >"$work/full/x"
rc "$gen" "$work/full"
if [ "$RC" = 1 ] && grep -q 'is not empty' "$work/err" && [ -f "$work/full/x" ] && [ "$(find "$work/full" -type f | wc -l | tr -d ' ')" = 1 ]; then ok "refuses_nonempty_without_force"; else nok "refuses_nonempty_without_force (rc $RC)"; fi

rc "$gen" --bogus "$work/none"
expect "unknown_arg_exit_2" 2 "$RC"

if $have_usign; then
  printf 'apt payload\n' >"$work/data"
  export GNUPGHOME="$work/gh" && mkdir -m 700 "$GNUPGHOME"
  gpg --batch --quiet --import "$keys/townjump-apt.sec.asc" 2>/dev/null
  gpg --batch --quiet --pinentry-mode loopback --passphrase '' --detach-sign -o "$work/data.sig" "$work/data" 2>/dev/null
  if gpgv --keyring "$keys/townjump.gpg" "$work/data.sig" "$work/data" >/dev/null 2>&1; then ok "public_key_verifies_signature_apt"; else nok "public_key_verifies_signature_apt"; fi
  gpgconf --homedir "$GNUPGHOME" --kill all >/dev/null 2>&1 || true
  unset GNUPGHOME

  "$usign_bin" -S -m "$work/data" -s "$keys/townjump-usign.sec" 2>/dev/null
  if "$usign_bin" -V -m "$work/data" -p "$keys/townjump-usign.pub" >/dev/null 2>&1; then ok "public_key_verifies_signature_openwrt"; else nok "public_key_verifies_signature_openwrt"; fi

  d="$(cat "$keys/deploy-dist.pub" "$keys/deploy-openwrt.pub" "$keys/deploy-keenetic.pub" | awk '{print $2}' | sort -u | wc -l | tr -d ' ')"
  c="$(cat "$keys"/deploy-*.pub | grep -c 'townjump-.* deploy key')"
  if [ "$d" = 3 ] && [ "$c" = 3 ]; then ok "deploy_keys_are_distinct"; else nok "deploy_keys_are_distinct ($d, $c)"; fi

  # Install into a scratch tree that is a git repository.
  tree="$work/tree" && mkdir "$tree" && git -C "$tree" init -q
  TOWNJUMP_KEYS_TREE="$tree" rc "$inst" "$keys"
  if [ "$RC" = 0 ] && cmp -s "$keys/townjump.gpg" "$tree/packaging/debian/keys/townjump.gpg" \
    && cmp -s "$keys/townjump-usign.pub" "$tree/packaging/openwrt/keys/townjump-usign.pub" \
    && cmp -s "$keys/townjump-apk.pub" "$tree/packaging/openwrt/keys/townjump-apk.pem"; then ok "install_public_keys_places_files"; else nok "install_public_keys_places_files (rc $RC)"; fi

  git -C "$tree" add -A
  if ! git -C "$tree" status --porcelain | grep -q '\.sec' && [ -z "$(find "$tree" -path "$tree/.git" -prune -o -name '*.sec*' -print)" ]; then
    ok "no_private_material_in_repo_after_install"; else nok "no_private_material_in_repo_after_install"; fi

  # A second set: swap one public key for a stranger's.
  other="$work/other"
  rc "$gen" "$other"
  mixed="$work/mixed" && cp -R "$keys" "$mixed" && cp "$other/townjump-usign.pub" "$mixed/townjump-usign.pub"
  tree2="$work/tree2" && mkdir "$tree2"
  TOWNJUMP_KEYS_TREE="$tree2" rc "$inst" "$mixed"
  if [ "$RC" = 1 ] && [ ! -e "$tree2/packaging" ]; then ok "install_rejects_mismatched_pair"; else nok "install_rejects_mismatched_pair (rc $RC)"; fi

  # The apt and apk public keys are checked against their secret keys too.
  for pair in "apt:townjump.gpg" "apk:townjump-apk.pub"; do
    kind="${pair%%:*}" f="${pair#*:}"
    mixed="$work/mixed-$kind" && cp -R "$keys" "$mixed" && cp "$other/$f" "$mixed/$f"
    treeX="$work/tree-$kind" && mkdir "$treeX"
    TOWNJUMP_KEYS_TREE="$treeX" rc "$inst" "$mixed"
    if [ "$RC" = 1 ] && [ ! -e "$treeX/packaging" ] && grep -q "$f does not match" "$work/err"; then
      ok "install_rejects_mismatched_pair_$kind"; else nok "install_rejects_mismatched_pair_$kind (rc $RC)"; fi
  done

  # --force replaces the whole set without asking anything.
  again="$work/again"
  rc "$gen" "$again"
  first_pub="$(cat "$again/deploy-dist.pub")"
  set +e; "$gen" --force "$again" >"$work/out" 2>"$work/err" </dev/null; RC=$?; set -e
  want="$(gpg --show-keys --with-colons "$again/townjump.gpg" | awk -F: '$1 == "fpr" { print $10; exit }')"
  if [ "$RC" = 0 ] && grep -qx "gpg-fingerprint: $want" "$again/fingerprints.txt" \
    && [ "$(cat "$again/deploy-dist.pub")" != "$first_pub" ]; then ok "force_regenerates_complete_set"; else nok "force_regenerates_complete_set (rc $RC)"; fi
fi

echo "1..$n"
[ "$fail" = 0 ]
