# OpenWrt packages

A signed package feed built by `packaging/openwrt/feed/build_feed.py`, without the OpenWrt SDK.

- OpenWrt 24.10 and older: `.ipk` for `opkg`, index signed with `usign`.
- OpenWrt 25.12 and newer: `.apk` (apk v3) for `apk`, index signed with an EC key.

The package holds `/usr/bin/townjump`, a static musl binary. It does not start anything.

Base address: `https://mesh-town.github.io/townjump/openwrt`. Address is provisional until the first publication.
No feed has been published yet; the commands below work after the first publication.

## Architectures

The feed directory is named after `DISTRIB_ARCH` of the router. Labels come from `packaging/openwrt/feed/targets.py`:
`aarch64_*` (cortex-a53, a72, a76, generic), `x86_64`, ARMv7 (`arm_cortex-a5`, `a7`, `a8`, `a9`, `a15` variants;
soft-float binary, the hard-float one only for the labels with an FPU in the name) and MIPS (`mips_24kc`,
`mipsel_24kc`, `mipsel_74kc`, `mipsel_24kc_24kf`). A label with no binary in the release is missing from the feed.

## Install

```sh
wget -O- https://mesh-town.github.io/townjump/openwrt/install.sh | sh
```

The script reads `/etc/openwrt_release`, picks `apk` or `opkg`, checks that the feed carries the architecture, installs
the key, adds the feed and installs (or upgrades) `townjump`. `TOWNJUMP_FEED=<address>` overrides the feed address.

## Install by hand

opkg (replace `<arch>` with `DISTRIB_ARCH`):

```sh
B=https://mesh-town.github.io/townjump/openwrt
wget -O /tmp/townjump-usign.pub "$B/townjump-usign.pub" && opkg-key add /tmp/townjump-usign.pub
echo "src/gz townjump $B/opkg/<arch>" >> /etc/opkg/customfeeds.conf
opkg update && opkg install townjump
```

apk:

```sh
B=https://mesh-town.github.io/townjump/openwrt
wget -O /etc/apk/keys/townjump.pem "$B/townjump-apk.pem"
echo "$B/apk/<arch>/packages.adb" >> /etc/apk/repositories.d/customfeeds.list
apk update && apk add townjump
```

## Update and remove

```sh
opkg update && opkg upgrade townjump     # apk: apk update && apk upgrade townjump
opkg remove townjump                     # apk: apk del townjump
```

Running `install.sh` again also upgrades.

## Run

townjump is not a service; the package installs no init script. By hand: `townjump --help`. To start it at boot, add
your own line to `/etc/rc.local` (before `exit 0`), for example `townjump <mode and arguments> &`, or write a procd
init script; the arguments are those of `townjump --help`.

## How it is built

`build_feed.py --fetched DIR --out DIR --version X.Y.Z [--pkg-release N] [--strict] [--only ipk|apk]`, with
`SOURCE_DATE_EPOCH` set and the signing keys in `OPENWRT_USIGN_SEC`, `OPENWRT_APK_SEC`, `OPENWRT_PUBKEYS`
(`--ephemeral-keys` for a dry run). The apk part uses the real `apk mkpkg`/`mkndx` (`feed/tools.sh` builds the tools).
Checks that do not use the builders: `scripts/dist-check/{ipk_check,adb_check}.py`. Public keys
(`townjump-usign.pub`, `townjump-apk.pem`) are committed to `packaging/openwrt/keys/` after the first publication.
