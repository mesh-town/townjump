#!/usr/bin/env bash
# Release asset helper: sums | check | release.
#   publish.sh sums <dir>
#   publish.sh check <version> <dir>
#   publish.sh release <version> <dir>
# Exit codes: 2 usage / bad version / missing dir, 1 other failure, 0 success.
set -euo pipefail

usage() {
  echo "usage: publish.sh sums <dir> | check <version> <dir> | release <version> <dir>" >&2
  exit 2
}

hash_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

valid_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
}

expected_assets() {
  local v="$1" t
  # Release assets: add new targets here
  local targets=(
    aarch64-apple-darwin
    x86_64-apple-darwin
    x86_64-unknown-linux-musl
    aarch64-unknown-linux-musl
    armv7-unknown-linux-musleabihf
    armv7-unknown-linux-musleabi
    mipsel-unknown-linux-musl
    mips-unknown-linux-musl
    aarch64-linux-android
    x86_64-pc-windows-msvc
  )
  for t in "${targets[@]}"; do
    if [[ "$t" == *windows* ]]; then
      echo "townjump-$v-$t.zip"
    else
      echo "townjump-$v-$t.tar.gz"
    fi
  done
  echo "townjump-$v.tar.gz"
  echo "townjump-setup-$v.exe"
}

cmd_sums() {
  local dir="$1" tmp f n=0
  [[ -d "$dir" ]] || { echo "no such directory: $dir" >&2; exit 2; }
  tmp="$(mktemp "${TMPDIR:-/tmp}/sums.XXXXXX")"
  trap 'rm -f "$tmp"' EXIT
  while IFS= read -r f; do
    printf '%s  %s\n' "$(hash_of "$dir/$f")" "$f" >>"$tmp"
    n=$((n + 1))
  done < <(cd "$dir" && find . -maxdepth 1 -type f \( -name '*.tar.gz' -o -name '*.zip' -o -name '*.exe' \) | sed 's|^\./||' | LC_ALL=C sort)
  if [[ "$n" -eq 0 ]]; then
    echo "no *.tar.gz, *.zip or *.exe files in $dir" >&2
    exit 1
  fi
  cat "$tmp" >"$dir/SHA256SUMS"
  rm -f "$tmp"
  trap - EXIT
}

cmd_check() {
  local v="$1" dir="$2" name cnt line want got bad=()
  valid_version "$v" || { echo "bad version: $v" >&2; exit 2; }
  [[ -d "$dir" ]] || { echo "no such directory: $dir" >&2; exit 2; }
  if [[ ! -f "$dir/SHA256SUMS" ]]; then
    echo "missing: SHA256SUMS" >&2
    exit 1
  fi
  while IFS= read -r name; do
    cnt="$(awk -v n="$name" '$2 == n' "$dir/SHA256SUMS" | wc -l | tr -d ' ')"
    if [[ "$cnt" != 1 ]]; then
      bad+=("$name (sums lines: $cnt)")
      continue
    fi
    if [[ ! -f "$dir/$name" ]]; then
      bad+=("$name (file missing)")
      continue
    fi
    line="$(awk -v n="$name" '$2 == n' "$dir/SHA256SUMS")"
    want="${line%% *}"
    got="$(hash_of "$dir/$name")"
    if [[ "$want" != "$got" ]]; then
      bad+=("$name (hash mismatch)")
    fi
  done < <(expected_assets "$v")
  if [[ "${#bad[@]}" -gt 0 ]]; then
    echo "release assets check failed:" >&2
    printf '  %s\n' "${bad[@]}" >&2
    exit 1
  fi
}

cmd_release() {
  local v="$1" dir="$2" repo args
  cmd_check "$v" "$dir"
  repo="${TOWNJUMP_DIST_REPO:-${GITHUB_REPOSITORY:-mesh-town/townjump}}"
  args=(release create "v$v" "$dir"/* --repo "$repo" --title "v$v" --generate-notes)
  if [[ -n "${GITHUB_REPOSITORY:-}" && "$repo" == "$GITHUB_REPOSITORY" ]]; then
    args+=(--verify-tag)
  elif [[ -z "${DIST_TOKEN:-}" ]]; then
    echo "::error::DIST_TOKEN is required to publish to $repo" >&2
    exit 1
  else
    export GH_TOKEN="$DIST_TOKEN"
  fi
  gh "${args[@]}"
}

[[ $# -ge 1 ]] || usage
sub="$1"
shift
case "$sub" in
  sums) [[ $# -eq 1 ]] || usage; cmd_sums "$1" ;;
  check) [[ $# -eq 2 ]] || usage; cmd_check "$1" "$2" ;;
  release)
    [[ $# -eq 2 ]] || usage
    valid_version "$1" || { echo "bad version: $1" >&2; exit 2; }
    [[ -d "$2" ]] || { echo "no such directory: $2" >&2; exit 2; }
    cmd_release "$1" "$2"
    ;;
  *) usage ;;
esac
