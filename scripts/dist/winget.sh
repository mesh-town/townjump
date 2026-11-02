#!/usr/bin/env bash
# winget.sh: manifests of the Windows installer for the winget community repository.
#   winget.sh render <version> <installer> <out> [--base-url URL]
#       writes <out>/manifests/m/MeshTown/Townjump/<version>/ (three files) from packaging/windows/winget/*.yaml.in.
#       The installer URL is <base>/townjump-setup-<version>.exe; the base is
#       https://github.com/<TOWNJUMP_DIST_REPO>/releases/download/v<version> unless --base-url is given.
#       A wrong version or a missing installer: exit 2, nothing written.
#   winget.sh submit <version>
#       Empty GITHUB_TOKEN: a notice, exit 0. komac not in PATH: exit 1.
#       Asks the GitHub API whether the package exists in microsoft/winget-pkgs:
#         404 -> a warning (the first pull request is made by hand from a fork), exit 0;
#         other than 200 -> an error, exit 1;
#         200 -> komac update MeshTown.Townjump --version <version> --urls <URL> --submit
#                (--dry-run when WINGET_DRY_RUN=1).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tpl="$here/../../packaging/windows/winget"
id=MeshTown.Townjump
manifest_version=1.10.0
repo="${TOWNJUMP_DIST_REPO:-mesh-town/townjump}"
api=https://api.github.com/repos/microsoft/winget-pkgs/contents/manifests/m/MeshTown/Townjump

usage() {
  echo "usage: winget.sh render <version> <installer> <out> [--base-url URL] | submit <version>" >&2
  exit 2
}

check_version() {
  [[ "${1:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
}

sha256_upper() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum -- "$1" | cut -d' ' -f1 | tr 'a-f' 'A-F'
  else
    shasum -a 256 -- "$1" | cut -d' ' -f1 | tr 'a-f' 'A-F'
  fi
}

# fill <template> <version> <url> <sha>: the template with the placeholders replaced.
fill() {
  local line
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line//@MANIFEST_VERSION@/$manifest_version}"
    line="${line//@VERSION@/$2}"
    line="${line//@INSTALLER_URL@/$3}"
    line="${line//@INSTALLER_SHA256@/$4}"
    printf '%s\n' "$line"
  done < "$1"
}

render() {
  [ "$#" -ge 3 ] || usage
  local version="$1" installer="$2" out="$3" base=""
  shift 3
  check_version "$version"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --base-url) [ "$#" -ge 2 ] || usage; base="$2"; shift 2 ;;
      *) usage ;;
    esac
  done
  [ -f "$installer" ] || { echo "winget.sh: no installer $installer" >&2; exit 2; }
  base="${base:-https://github.com/$repo/releases/download/v$version}"
  local url="${base%/}/townjump-setup-$version.exe" sha dir name
  sha="$(sha256_upper "$installer")"
  dir="$out/manifests/m/MeshTown/Townjump/$version"
  mkdir -p "$dir"
  for name in "$id" "$id.locale.en-US" "$id.installer"; do
    fill "$tpl/$name.yaml.in" "$version" "$url" "$sha" > "$dir/$name.yaml"
  done
}

submit() {
  [ "$#" -eq 1 ] || usage
  check_version "$1"
  if [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "::notice::GITHUB_TOKEN is not set; no winget pull request is made"
    return 0
  fi
  command -v komac > /dev/null 2>&1 || { echo "winget.sh: komac not found in PATH" >&2; exit 1; }
  local code
  code="$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $GITHUB_TOKEN" "$api")" || code=000
  case "$code" in
    200) ;;
    404)
      echo "::warning::$id is not in microsoft/winget-pkgs yet; make the first pull request by hand from a fork (manifests: winget.sh render)"
      return 0
      ;;
    *)
      echo "::error::winget-pkgs API answered $code for $id"
      exit 1
      ;;
  esac
  local mode=--submit
  [ "${WINGET_DRY_RUN:-0}" = 1 ] && mode=--dry-run
  komac update "$id" --version "$1" --urls "https://github.com/$repo/releases/download/v$1/townjump-setup-$1.exe" "$mode"
}

case "${1:-}" in
  render) shift; render "$@" ;;
  submit) shift; submit "$@" ;;
  *) usage ;;
esac
