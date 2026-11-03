# Debian and Ubuntu packages

`townjump_<version>-<n>_<arch>.deb` for `amd64`, `arm64` and `armhf` (the `armhf` package carries the static
hard-float ARMv7 binary). The package contains:

- `/usr/bin/townjump`, a static musl binary;
- `/usr/share/doc/townjump/copyright` and `README.md`;
- `/etc/apt/sources.list.d/townjump.sources` and `/usr/share/keyrings/townjump.gpg`, so that `apt upgrade` finds new
  versions (only in packages built together with the apt key).

There is no service: townjump is started by hand (or by your own unit).

Base address: `https://mesh-town.github.io/townjump`. Address is provisional until the first publication.
No package has been published yet; the commands below work after the first publication.

## Install

```sh
arch="$(dpkg --print-architecture)"
curl -fLO "https://mesh-town.github.io/townjump/apt/townjump_${arch}.deb"
sudo apt install "./townjump_${arch}.deb"
```

## Update and remove

```sh
sudo apt update && sudo apt upgrade     # townjump comes from the source the package installed
sudo apt remove townjump                # `purge` is the same: the package has no configuration files
```

## Run

```sh
townjump --help
```

`--help` lists the server and client modes (share a local port and get a key; connect with a key).

## How it is built

- Workflow `.github/workflows/deb.yml`: called from the release workflow after the release exists, or started by hand
  (`workflow_dispatch`; `publish` is `false` by default, so a manual run is a dry run).
- `scripts/dist/deb-build.sh <fetched> <out> <version> <pkg-release>` builds the packages from the release archives
  (checked against `SHA256SUMS`); `packaging/debian/deb.py` writes one `.deb`, `packaging/debian/apt_repo.py` keeps the
  signed apt repository (`pool/`, `dists/stable/`, `InRelease`, `Release.gpg`).
- The build is reproducible: it needs `SOURCE_DATE_EPOCH`, building twice gives the same bytes.
- Checks that do not use the builders: `scripts/dist-check/{deb_check,apt_check}.py`.

## Signature and key

The repository is signed with an OpenPGP key (ed25519). The public key is installed by the package into
`/usr/share/keyrings/townjump.gpg` and is committed as `packaging/debian/keys/townjump.gpg` after the first publication.
Without the signing secret the workflow builds `.deb` files without the apt source and publishes nothing.
