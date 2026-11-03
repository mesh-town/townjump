#!/usr/bin/env bash
# deb-update.sh <apt-dir> <debs-dir>
# Adds the .deb files in <debs-dir> to the apt repository in <apt-dir> (created when absent), keeping four versions per
# architecture, signs it with the secret key file TOWNJUMP_APT_KEY and checks the result against the public keyring.
# Public keyring: TOWNJUMP_APT_PUBKEY if set, else the committed packaging/debian/keys/townjump.gpg.
# Exit codes: 0 done, 1 update or check failed, 2 bad arguments or no key.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: deb-update.sh <apt-dir> <debs-dir>" >&2
  exit 2
fi
apt_dir="$1" debs="$2"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$here/../.."
key="${TOWNJUMP_APT_KEY:-}"
pub="${TOWNJUMP_APT_PUBKEY:-$root/packaging/debian/keys/townjump.gpg}"
if [ -z "$key" ] || [ ! -f "$key" ]; then
  echo "deb-update.sh: TOWNJUMP_APT_KEY must name the secret key file" >&2
  exit 2
fi
if [ ! -f "$pub" ]; then
  echo "deb-update.sh: public keyring '$pub' not found" >&2
  exit 2
fi
python3 "$root/packaging/debian/apt_repo.py" update "$apt_dir" "$debs" --keep 4 --key "$key" --pubkey "$pub"
python3 "$root/scripts/dist-check/apt_check.py" "$apt_dir" --pubkey "$pub"
