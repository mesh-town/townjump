#!/usr/bin/env bash
# Installs the reference JavaScript Holesail (npm `holesail@2.4.4`) into <code root>/.tmp/npm-holesail,
# the tree the interop tests and probes look for (see interop/README.md and docs/testing.md).
# The dependency tree is pinned by interop/js/holesail-2.4.4.package-lock.json.
# Requires Node.js and npm. Idempotent: rerunning on an installed tree changes nothing.
set -euo pipefail

CODE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOLESAIL_VERSION="${HOLESAIL_VERSION:-2.4.4}"
LOCK="$CODE/interop/js/holesail-$HOLESAIL_VERSION.package-lock.json"
NPM_DIR="$CODE/.tmp/npm-holesail"

log() { printf '[setup-js] %s\n' "$*" >&2; }

command -v node >/dev/null || { log "node not found in PATH"; exit 1; }
command -v npm >/dev/null || { log "npm not found in PATH"; exit 1; }

mkdir -p "$NPM_DIR"
cd "$NPM_DIR"
[[ -f package.json ]] || npm init -y >/dev/null
if [[ ! -f package-lock.json ]]; then
  if [[ -f "$LOCK" ]]; then
    cp "$LOCK" package-lock.json
  else
    log "no lock file at $LOCK; resolving versions from the registry"
  fi
fi
npm install "holesail@$HOLESAIL_VERSION" --no-audit --no-fund --ignore-scripts 2>&1 | tail -n 3 >&2
log "installed: $(ls node_modules | wc -l | tr -d ' ') entries in $NPM_DIR/node_modules"
log "to use from another checkout: export TOWNJUMP_JS_NM=$NPM_DIR/node_modules"
