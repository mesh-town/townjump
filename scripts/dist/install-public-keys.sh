#!/usr/bin/env bash
# Put the public signing keys from a gen-keys.sh directory where the builders read them, after checking
# that each public key matches its secret key (a test signature). Secret keys are never copied.
# Usage: install-public-keys.sh <keys-dir>
# Environment: USIGN (usign binary, else PATH); TOWNJUMP_KEYS_TREE (target tree, default: this repository).
set -euo pipefail

if [ "$#" -ne 1 ] || [ "${1#-}" != "$1" ]; then
  echo "usage: install-public-keys.sh <keys-dir>" >&2
  exit 2
fi
src="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tree="${TOWNJUMP_KEYS_TREE:-$here/../..}"
[ -d "$src" ] || { echo "install-public-keys.sh: '$src' is not a directory" >&2; exit 2; }
for f in townjump-apt.sec.asc townjump.gpg townjump-usign.sec townjump-usign.pub townjump-apk.sec townjump-apk.pub; do
  [ -f "$src/$f" ] || { echo "install-public-keys.sh: $src/$f is missing" >&2; exit 2; }
done
usign_bin="${USIGN:-$(command -v usign || true)}"
missing=""
for t in gpg gpgv openssl; do command -v "$t" >/dev/null 2>&1 || missing="$missing $t"; done
{ [ -n "$usign_bin" ] && [ -x "$usign_bin" ]; } || missing="$missing usign"
[ -z "$missing" ] || { echo "install-public-keys.sh: required tools not found:$missing" >&2; exit 1; }

work="$(mktemp -d /tmp/townjump-pubkeys.XXXXXX)"
trap 'gpgconf --homedir "$work/gnupg" --kill all >/dev/null 2>&1 || true; rm -rf "$work"' EXIT
printf 'townjump key check\n' >"$work/probe"

# apt: sign with the imported secret key, verify with the public keyring.
mkdir -m 700 "$work/gnupg"
gpg --homedir "$work/gnupg" --batch --quiet --import "$src/townjump-apt.sec.asc" 2>/dev/null
gpg --homedir "$work/gnupg" --batch --quiet --pinentry-mode loopback --passphrase '' \
  --detach-sign -o "$work/probe.gpgsig" "$work/probe" 2>/dev/null
gpgv --keyring "$src/townjump.gpg" "$work/probe.gpgsig" "$work/probe" >/dev/null 2>&1 \
  || { echo "install-public-keys.sh: townjump.gpg does not match townjump-apt.sec.asc" >&2; exit 1; }

# usign
"$usign_bin" -S -m "$work/probe" -s "$src/townjump-usign.sec" 2>/dev/null
"$usign_bin" -V -m "$work/probe" -p "$src/townjump-usign.pub" >/dev/null 2>&1 \
  || { echo "install-public-keys.sh: townjump-usign.pub does not match townjump-usign.sec" >&2; exit 1; }

# apk: the public half of the secret must equal the public file
derived="$(openssl pkey -in "$src/townjump-apk.sec" -pubout 2>/dev/null)" || derived=""
given="$(openssl pkey -pubin -in "$src/townjump-apk.pub" -pubout 2>/dev/null)" || given=""
if [ -z "$derived" ] || [ "$derived" != "$given" ]; then
  echo "install-public-keys.sh: townjump-apk.pub does not match townjump-apk.sec" >&2
  exit 1
fi

install -d "$tree/packaging/debian/keys" "$tree/packaging/openwrt/keys"
install -m 644 "$src/townjump.gpg" "$tree/packaging/debian/keys/townjump.gpg"
install -m 644 "$src/townjump-usign.pub" "$tree/packaging/openwrt/keys/townjump-usign.pub"
install -m 644 "$src/townjump-apk.pub" "$tree/packaging/openwrt/keys/townjump-apk.pem"
echo "installed: packaging/debian/keys/townjump.gpg packaging/openwrt/keys/townjump-usign.pub packaging/openwrt/keys/townjump-apk.pem"
