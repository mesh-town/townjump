# Image of the statically linked townjump binary. The build context is the directory made by
# scripts/dist/container-ctx.sh (linux/<arch>/townjump). There is no shell and there are no CA certificates.
FROM scratch

ARG TARGETOS
ARG TARGETARCH
ARG TARGETVARIANT
ARG VERSION=unknown

LABEL org.opencontainers.image.source="https://github.com/mesh-town/townjump" \
      org.opencontainers.image.title="townjump" \
      org.opencontainers.image.description="Peer-to-peer tunnels over the Holesail network, in Rust" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}"

COPY ${TARGETOS}/${TARGETARCH}${TARGETVARIANT}/townjump /usr/bin/townjump

ENV HOME=/data
# WORKDIR creates /data without a RUN step
WORKDIR /data
VOLUME /data

ENTRYPOINT ["/usr/bin/townjump"]
CMD ["--help"]
