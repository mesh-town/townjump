#!/usr/bin/env bash
# tools.sh <dir> - builds the two tools the feed builder needs, at pinned commits:
#   <dir>/bin/apk    apk-tools v3.0.8 (mkpkg, mkndx, adbsign)
#   <dir>/bin/usign  usign (signs and verifies the opkg index)
# A second call with both binaries in place builds nothing. Linux only.
#
# Packages to install first (Ubuntu 24.04):
#   apt-get install -y git gcc meson ninja-build pkg-config libssl-dev zlib1g-dev libzstd-dev fakeroot openssl
#
# apk is linked dynamically against glibc on purpose: fakeroot works through LD_PRELOAD and must see it.
# libapk is built static (-Ddefault_library=static), so no LD_LIBRARY_PATH is needed.
set -euo pipefail

# The GitHub mirror of gitlab.alpinelinux.org/alpine/apk-tools: the GitLab host answers CI runners with HTTP 418.
APK_URL="https://github.com/alpinelinux/apk-tools.git"
APK_COMMIT="44dcdfc255c26a4e19a4a27f66a54169cce5ca72"
USIGN_URL="https://git.openwrt.org/project/usign.git"
USIGN_COMMIT="c4c72b1b07945ee192361dc751291a7c98d6adcd"

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "usage: tools.sh <dir>" >&2
  exit 2
fi
if [ "$(uname -s)" != Linux ]; then
  echo "tools.sh: Linux only" >&2
  exit 2
fi

mkdir -p "$1"
dir="$(cd "$1" && pwd)"
bin="$dir/bin"
mkdir -p "$bin"

if [ -x "$bin/apk" ] && [ -x "$bin/usign" ]; then
  echo "tools.sh: apk and usign already built in $bin"
  exit 0
fi

# fetch <url> <commit> <dest>: a clone at exactly <commit>, verified.
fetch() {
  local url="$1" commit="$2" dest="$3"
  rm -rf "$dest"
  git init -q "$dest"
  git -C "$dest" remote add origin "$url"
  git -C "$dest" fetch -q --depth 1 origin "$commit"
  git -C "$dest" checkout -q FETCH_HEAD
  if [ "$(git -C "$dest" rev-parse HEAD)" != "$commit" ]; then
    echo "tools.sh: $url is not at $commit" >&2
    exit 1
  fi
}

if [ ! -x "$bin/usign" ]; then
  fetch "$USIGN_URL" "$USIGN_COMMIT" "$dir/src/usign"
  (
    cd "$dir/src/usign"
    cc -O2 --std=gnu99 -o "$bin/usign" ed25519.c edsign.c f25519.c fprime.c sha512.c main.c base64.c
  )
fi

if [ ! -x "$bin/apk" ]; then
  fetch "$APK_URL" "$APK_COMMIT" "$dir/src/apk-tools"
  (
    cd "$dir/src/apk-tools"
    meson setup "$dir/build-apk" \
      -Dhelp=disabled -Ddocs=disabled -Dlua=disabled -Dpython=disabled -Dtests=disabled \
      -Dzstd=enabled -Ddefault_library=static
    ninja -C "$dir/build-apk"
  )
  cp "$dir/build-apk/src/apk" "$bin/apk"
fi

# apk runs without LD_LIBRARY_PATH; if it does not, the wrapper below is the fallback.
if ! "$bin/apk" --version > /dev/null 2>&1; then
  real="$bin/apk.real"
  mv "$bin/apk" "$real"
  libdir="$(dirname "$(find "$dir/build-apk" -name 'libapk.so*' | head -n 1)")"
  printf '#!/bin/sh\nLD_LIBRARY_PATH=%s exec %s "$@"\n' "$libdir" "$real" > "$bin/apk"
  chmod 755 "$bin/apk"
fi

"$bin/apk" --version
"$bin/usign" 2>&1 | head -n 1 || true
echo "tools.sh: done"
