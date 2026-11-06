#!/usr/bin/env bash
# Generate every key the publication needs, in one directory that must be OUTSIDE any git repository.
# Usage: gen-keys.sh [--force] <dir>
# Creates: GPG (apt), usign + apk (OpenWrt) signing keys and three SSH deploy keys, then prints which file
# goes into which secret of which repository (the keys themselves are never printed).
# Environment: USIGN (path to the usign binary, else PATH).
set -euo pipefail

IDENTITY='Mesh Town <340663027+mesh-town@users.noreply.github.com>'
usage() { echo "usage: gen-keys.sh [--force] <dir-outside-any-git-repository>" >&2; }

force=false dir=""
for a in "$@"; do
  case "$a" in
    --force) force=true ;;
    -h | --help) usage; exit 0 ;;
    -*) echo "gen-keys.sh: unknown argument '$a'" >&2; usage; exit 2 ;;
    *)
      if [ -n "$dir" ]; then echo "gen-keys.sh: more than one directory" >&2; usage; exit 2; fi
      dir="$a"
      ;;
  esac
done
if [ -z "$dir" ]; then usage; exit 2; fi

# The directory must not be inside a git repository (look at the nearest existing ancestor).
probe="$dir"
while [ ! -d "$probe" ]; do
  parent="$(dirname "$probe")"
  [ "$parent" != "$probe" ] || break
  probe="$parent"
done
if git -C "$probe" rev-parse --git-dir >/dev/null 2>&1; then
  echo "gen-keys.sh: '$dir' is inside a git repository; secret keys must live outside any repository" >&2
  exit 1
fi
if [ -d "$dir" ] && [ -n "$(ls -A "$dir")" ] && ! $force; then
  echo "gen-keys.sh: '$dir' is not empty (use --force to overwrite)" >&2
  exit 1
fi

usign_bin="${USIGN:-$(command -v usign || true)}"
missing=""
for t in gpg openssl ssh-keygen; do command -v "$t" >/dev/null 2>&1 || missing="$missing $t"; done
{ [ -n "$usign_bin" ] && [ -x "$usign_bin" ]; } || missing="$missing usign"
if [ -n "$missing" ]; then
  echo "gen-keys.sh: required tools not found:$missing (usign: set USIGN=<path> or build it with packaging/openwrt/feed/tools.sh)" >&2
  exit 1
fi

umask 077
mkdir -p "$dir"
chmod 700 "$dir"
dir="$(cd "$dir" && pwd)"
if $force; then
  # Remove only the files of a previous set, so ssh-keygen does not stop to ask about overwriting.
  rm -f "$dir"/townjump-apt.sec.asc "$dir"/townjump.gpg "$dir"/townjump.asc "$dir"/townjump-apt.rev \
    "$dir"/townjump-usign.sec "$dir"/townjump-usign.pub "$dir"/townjump-apk.sec "$dir"/townjump-apk.pub \
    "$dir"/deploy-dist "$dir"/deploy-dist.pub "$dir"/deploy-openwrt "$dir"/deploy-openwrt.pub \
    "$dir"/deploy-keenetic "$dir"/deploy-keenetic.pub "$dir"/fingerprints.txt
fi

work="$(mktemp -d /tmp/townjump-keys.XXXXXX)"
trap 'gpgconf --homedir "$work/gnupg" --kill all >/dev/null 2>&1 || true; rm -rf "$work"' EXIT

# GPG (apt): ed25519, no expiry, no passphrase, in a throwaway home.
export GNUPGHOME="$work/gnupg"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
gpg --batch --quiet --pinentry-mode loopback --passphrase '' \
  --quick-generate-key "$IDENTITY" ed25519 sign never 2>/dev/null
fpr="$(gpg --batch --with-colons --fingerprint 2>/dev/null | awk -F: '$1 == "fpr" { print $10; exit }')"
gpg --batch --pinentry-mode loopback --passphrase '' --armor --export-secret-keys "$fpr" >"$dir/townjump-apt.sec.asc"
gpg --batch --export "$fpr" >"$dir/townjump.gpg"
gpg --batch --armor --export "$fpr" >"$dir/townjump.asc"
rev="$GNUPGHOME/openpgp-revocs.d/$fpr.rev"
[ -f "$rev" ] || { echo "gen-keys.sh: gpg wrote no revocation certificate" >&2; exit 1; }
cp "$rev" "$dir/townjump-apt.rev"

# usign (opkg index) and apk (P-256) signing keys.
"$usign_bin" -G -c "Mesh Town OpenWrt feed" -p "$dir/townjump-usign.pub" -s "$dir/townjump-usign.sec" 2>/dev/null
openssl ecparam -name prime256v1 -genkey -noout -out "$dir/townjump-apk.sec"
openssl pkey -in "$dir/townjump-apk.sec" -pubout -out "$dir/townjump-apk.pub"

# Deploy keys: one per distribution repository.
for n in dist openwrt keenetic; do
  ssh-keygen -q -t ed25519 -N '' -C "townjump-$n deploy key" -f "$dir/deploy-$n"
done

{
  echo "gpg-fingerprint: $fpr"
  echo "identity: $IDENTITY"
  for f in townjump.gpg townjump.asc townjump-usign.pub townjump-apk.pub deploy-dist.pub deploy-openwrt.pub deploy-keenetic.pub; do
    printf 'sha256 %s  %s\n' "$(openssl dgst -sha256 -r "$dir/$f" | cut -d' ' -f1)" "$f"
  done
} >"$dir/fingerprints.txt"

find "$dir" -type f -exec chmod 600 {} +
chmod 700 "$dir"

cat <<TABLE
Keys written to: $dir  (GPG fingerprint $fpr)

Secrets for the repository mesh-town/townjump (Settings -> Secrets and variables -> Actions):
  file                      secret
  townjump-apt.sec.asc      APT_GPG_KEY
  townjump-usign.sec        OPENWRT_USIGN_KEY
  townjump-apk.sec          OPENWRT_APK_KEY
  deploy-dist               DIST_DEPLOY_KEY
  deploy-openwrt            OPENWRT_DEPLOY_KEY
  deploy-keenetic           KEENETIC_DEPLOY_KEY

Public halves of the deploy keys (add as a deploy key WITH write access):
  deploy-dist.pub           -> mesh-town/townjump-dist
  deploy-openwrt.pub        -> mesh-town/townjump-openwrt
  deploy-keenetic.pub       -> mesh-town/townjump-keenetic

Public keys for the tree: scripts/dist/install-public-keys.sh '$dir'
Keep townjump-apt.rev (revocation certificate) and a backup of the whole directory offline.
TABLE
