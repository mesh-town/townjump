# Keenetic (Entware) packages

An unsigned `opkg` feed for routers that run Entware, built by `packaging/keenetic/build_feed.py`. The package holds
`/opt/bin/townjump`, a static musl binary. It does not start anything.

Base address: `https://mesh-town.github.io/townjump-keenetic`. The feed is the whole content of the
`mesh-town/townjump-keenetic` repository (`install.sh` in its root), published with the secret `KEENETIC_DEPLOY_KEY`;
the repository is generated and not edited by hand.

## Architectures

The feed directory is named after the Entware architecture:

| Entware directory | Rust target |
|---|---|
| `aarch64-k3.10` | `aarch64-unknown-linux-musl` |
| `mipselsf-k3.4` | `mipsel-unknown-linux-musl` (when `MIPS_ENABLED` is on) |
| `mipssf-k3.4` | `mips-unknown-linux-musl` (when `MIPS_ENABLED` is on) |

## Install

Entware must be installed first (`/opt/bin/opkg`). In an SSH session on the router:

```sh
wget -O- https://mesh-town.github.io/townjump-keenetic/install.sh | sh
```

The script takes the architecture from the `src/gz entware` line of `/opt/etc/opkg.conf`, checks that the feed has it,
installs `wget-ssl` and `ca-certificates` if the default `wget` cannot open https, writes
`/opt/etc/opkg/townjump.conf` and installs (or upgrades) `townjump`. `TOWNJUMP_FEED=<address>` overrides the address.

## Update and remove

```sh
/opt/bin/opkg update && /opt/bin/opkg upgrade townjump
/opt/bin/opkg remove townjump
rm /opt/etc/opkg/townjump.conf     # drop the feed line as well
```

## Limits

- The feed is not signed; integrity comes from https only.
- Old kernels (3.x) on the MIPS models: use the binary only after a trial run with `townjump --help`.
- There is no service; start townjump by hand or from your own script in `/opt/etc/init.d/`.
- Only the architectures in the table are published.
