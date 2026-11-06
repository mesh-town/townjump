# Container image

`ghcr.io/mesh-town/townjump` holds the statically linked (musl) `townjump` binary and nothing else: no shell, no
package manager, no CA certificates. Platforms: `linux/amd64`, `linux/arm64`, `linux/arm/v7`.

Tags: `<version>` (for example `0.1.19`) and `latest`. They are pushed by the `container-image` workflow only for a
`v*` tag.

## Run

    docker run --rm ghcr.io/mesh-town/townjump:latest --help

The arguments after the image name are the CLI arguments (see the project README for the commands). The entrypoint is
`/usr/bin/townjump`, the default command is `--help`. `HOME` and the working directory are `/data`, a volume: mount one
to keep keys and state, for example `-v townjump-data:/data`.

The container needs no special network mode to join the public DHT: the default bridge/NAT network is enough (checked
with Apple `container`; the Docker host network was not measured). `--network host` is only expected to matter for LAN
discovery, which uses multicast and does not cross a bridge.

## Build the context by hand

The Dockerfile takes the binaries from a context made out of the release archives:

    gh release download v<version> --repo mesh-town/townjump --dir assets \
      --pattern 'townjump-<version>-*-linux-musl*.tar.gz' --pattern SHA256SUMS
    scripts/dist/container-ctx.sh assets ctx <version>
    docker buildx build --platform linux/amd64,linux/arm64,linux/arm/v7 \
      --build-arg VERSION=<version> --file packaging/container/Dockerfile ctx

`container-ctx.sh` checks every archive against `SHA256SUMS` and writes `ctx/linux/{amd64,arm64,armv7}/townjump`
(mode 0755). The Dockerfile selects the file by `TARGETOS/TARGETARCH+TARGETVARIANT`, so `linux/arm/v7` reads
`linux/armv7`. The Apple builder (`container build`) sets `TARGETVARIANT=v8` for arm64; add
`--build-arg TARGETVARIANT=`.
