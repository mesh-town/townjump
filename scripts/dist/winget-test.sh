#!/usr/bin/env bash
# winget-test.sh: runs winget.sh render/submit (submit with fake `komac` and `curl` in PATH); TAP output.
# ok/nok always return 0, so "A && ok || nok" is a true if-then-else here.
# shellcheck disable=SC2015,SC2012
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
n=0
bad=0
ok() { n=$((n + 1)); echo "ok $n - $1"; }
nok() { n=$((n + 1)); bad=$((bad + 1)); echo "not ok $n - $1"; }
# check <name> <command...>: ok when the command succeeds.
check() { local name="$1"; shift; if "$@" > /dev/null 2>&1; then ok "$name"; else nok "$name"; fi; }

mkdir -p "$tmp/bin"
cat > "$tmp/bin/komac" <<'K'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_LOG"
K
cat > "$tmp/bin/curl" <<'K'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_CURL_LOG"
printf '%s' "$FAKE_HTTP"
K
chmod +x "$tmp/bin/komac" "$tmp/bin/curl"
printf 'installer bytes\n' > "$tmp/setup.exe"
want_sha="$(shasum -a 256 "$tmp/setup.exe" | cut -d' ' -f1 | tr 'a-f' 'A-F')"
url=https://github.com/mesh-town/townjump/releases/download/v1.2.3/townjump-setup-1.2.3.exe

unset GITHUB_TOKEN WINGET_DRY_RUN TOWNJUMP_DIST_REPO
export FAKE_LOG="$tmp/log" FAKE_CURL_LOG="$tmp/curllog" FAKE_HTTP=200
: > "$FAKE_LOG"
: > "$FAKE_CURL_LOG"
base_path="$PATH"

dir="$tmp/out/manifests/m/MeshTown/Townjump/1.2.3"
rc=0
"$here/winget.sh" render 1.2.3 "$tmp/setup.exe" "$tmp/out" > /dev/null 2>&1 || rc=$?
[ "$rc" = 0 ] && ok "render exits 0" || nok "render exits 0 (got $rc)"
[ "$(ls "$dir" | wc -l | tr -d ' ')" = 3 ] && ok "render: three files" || nok "render: three files"
names="MeshTown.Townjump.yaml MeshTown.Townjump.locale.en-US.yaml MeshTown.Townjump.installer.yaml"
for f in $names; do
  check "$f: PackageVersion line" grep -qx 'PackageVersion: 1.2.3' "$dir/$f"
  if grep -Eq '@[A-Z_]+@' "$dir/$f"; then nok "$f: no placeholder left"; else ok "$f: no placeholder left"; fi
  check "$f: ManifestVersion 1.10.0" grep -qx 'ManifestVersion: 1.10.0' "$dir/$f"
done
for pair in MeshTown.Townjump.yaml:version MeshTown.Townjump.locale.en-US.yaml:defaultLocale \
  MeshTown.Townjump.installer.yaml:installer; do
  check "${pair%%:*}: schema header" \
    grep -qxF "# yaml-language-server: \$schema=https://aka.ms/winget-manifest.${pair#*:}.1.10.0.schema.json" "$dir/${pair%%:*}"
done
inst="$dir/MeshTown.Townjump.installer.yaml"
check "InstallerSha256 is the upper-case sha of the file" grep -qx "  InstallerSha256: $want_sha" "$inst"
check "default InstallerUrl" grep -qx "  InstallerUrl: $url" "$inst"
check "ProductCode" grep -qx '  ProductCode: Townjump' "$inst"
check "Commands" grep -qx -- '- townjump' "$inst"
check "locale: PackageName" grep -qx 'PackageName: Townjump' "$dir/MeshTown.Townjump.locale.en-US.yaml"

"$here/winget.sh" render 1.2.3 "$tmp/setup.exe" "$tmp/out2" --base-url https://example.test/dl/ > /dev/null 2>&1 || true
check "--base-url changes the url" grep -qx '  InstallerUrl: https://example.test/dl/townjump-setup-1.2.3.exe' \
  "$tmp/out2/manifests/m/MeshTown/Townjump/1.2.3/MeshTown.Townjump.installer.yaml"
TOWNJUMP_DIST_REPO=o/r "$here/winget.sh" render 1.2.3 "$tmp/setup.exe" "$tmp/out3" > /dev/null 2>&1 || true
check "TOWNJUMP_DIST_REPO changes the url" grep -qx \
  '  InstallerUrl: https://github.com/o/r/releases/download/v1.2.3/townjump-setup-1.2.3.exe' \
  "$tmp/out3/manifests/m/MeshTown/Townjump/1.2.3/MeshTown.Townjump.installer.yaml"

# refusals: exit 2 and nothing written
refuse() { # refuse <what> <args...>
  local what="$1" r=0
  shift
  rm -rf "$tmp/bad"
  "$here/winget.sh" render "$@" "$tmp/bad" > /dev/null 2>&1 || r=$?
  [ "$r" = 2 ] && ok "$what: exit 2" || nok "$what: exit 2 (got $r)"
  [ ! -e "$tmp/bad" ] && ok "$what: nothing written" || nok "$what: nothing written"
}
refuse "version 1.2" 1.2 "$tmp/setup.exe"
refuse "no installer" 1.2.3 "$tmp/missing.exe"
r=0
"$here/winget.sh" render 1.2.3 "$tmp/setup.exe" "$tmp/bad2" --bogus > /dev/null 2>&1 || r=$?
[ "$r" = 2 ] && ok "unknown render option: exit 2" || nok "unknown render option: exit 2 (got $r)"
r=0
"$here/winget.sh" > /dev/null 2>&1 || r=$?
[ "$r" = 2 ] && ok "no command: exit 2" || nok "no command: exit 2 (got $r)"
r=0
"$here/winget.sh" submit 1.2 > /dev/null 2>&1 || r=$?
[ "$r" = 2 ] && ok "submit bad version: exit 2" || nok "submit bad version: exit 2 (got $r)"

# submit
sub() { rc=0; out="$(PATH="$tmp/bin:$base_path" "$here/winget.sh" submit "$@" 2>&1)" || rc=$?; }
: > "$FAKE_LOG"
sub 1.2.3
[ "$rc" = 0 ] && ok "no token: exit 0" || nok "no token: exit 0 (got $rc)"
[ ! -s "$FAKE_LOG" ] && ok "no token: komac not called" || nok "no token: komac not called"
[ ! -s "$FAKE_CURL_LOG" ] && ok "no token: curl not called" || nok "no token: curl not called"
case "$out" in *::notice::*) ok "no token: notice" ;; *) nok "no token: notice" ;; esac

export GITHUB_TOKEN=t
r=0
PATH="/usr/bin:/bin" "$here/winget.sh" submit 1.2.3 > /dev/null 2>&1 || r=$?
[ "$r" = 1 ] && ok "no komac: exit 1" || nok "no komac: exit 1 (got $r)"

: > "$FAKE_LOG"
FAKE_HTTP=404 sub 1.2.3
[ "$rc" = 0 ] && ok "404: exit 0" || nok "404: exit 0 (got $rc)"
case "$out" in *::warning::*) ok "404: warning" ;; *) nok "404: warning" ;; esac
[ ! -s "$FAKE_LOG" ] && ok "404: komac not called" || nok "404: komac not called"

: > "$FAKE_LOG"
FAKE_HTTP=200 sub 1.2.3
[ "$rc" = 0 ] && ok "200: exit 0" || nok "200: exit 0 (got $rc)"
[ "$(cat "$FAKE_LOG")" = "update MeshTown.Townjump --version 1.2.3 --urls $url --submit" ] \
  && ok "200: komac argv" || nok "200: komac argv: $(cat "$FAKE_LOG")"
grep -q 'api.github.com/repos/microsoft/winget-pkgs/contents/manifests/m/MeshTown/Townjump' "$FAKE_CURL_LOG" \
  && ok "curl asks the winget-pkgs API" || nok "curl asks the winget-pkgs API"

: > "$FAKE_LOG"
FAKE_HTTP=200 WINGET_DRY_RUN=1 sub 1.2.3
[ "$(cat "$FAKE_LOG")" = "update MeshTown.Townjump --version 1.2.3 --urls $url --dry-run" ] \
  && ok "dry run: komac argv" || nok "dry run: komac argv: $(cat "$FAKE_LOG")"

: > "$FAKE_LOG"
FAKE_HTTP=500 sub 1.2.3
[ "$rc" = 1 ] && ok "500: exit 1" || nok "500: exit 1 (got $rc)"
case "$out" in *::error::*) ok "500: error annotation" ;; *) nok "500: error annotation" ;; esac
[ ! -s "$FAKE_LOG" ] && ok "500: komac not called" || nok "500: komac not called"

# metadata: one description phrase, landing page URLs, licence link stays on GitHub
loc="$dir/MeshTown.Townjump.locale.en-US.yaml"
landing=https://mesh-town.github.io/townjump/
short="$(sed -n "s/^ShortDescription: '\(.*\)'$/\1/p" "$loc")"
[ "$short" = "Single-binary Rust port of Holesail: share a local port over a peer-to-peer tunnel, reach it anywhere with one key. No server of your own." ] && [ "${#short}" -le 256 ] \
  && ok "description_is_the_one_phrase" || nok "description_is_the_one_phrase"
check "description_is_the_one_phrase: Description present" grep -q '^Description: ' "$loc"
if grep -Eiq 'vpn|relay server' "$loc"; then nok "no_vpn_or_relay_server_words"; else ok "no_vpn_or_relay_server_words"; fi
check "urls_point_to_landing: PackageUrl" grep -qx "PackageUrl: $landing" "$loc"
check "urls_point_to_landing: PublisherUrl" grep -qx "PublisherUrl: $landing" "$loc"
check "license_url_unchanged" grep -qx 'LicenseUrl: https://github.com/mesh-town/townjump/blob/main/LICENSE' "$loc"

echo "1..$n"
[ "$bad" = 0 ]
