# pixel-cast images, built from the static Linux binaries of a release (see .github/workflows/release.yml):
# docker/<arch>/pixel-cast must hold the binary of each platform built (amd64, arm64).
#
#   minimal: the binary alone (no shell, no ffmpeg): patterns, images, GIFs
#   ffmpeg:  Alpine with ffmpeg, for videos too
#
# Target file: /config/pixel-cast/targets.toml (mount /config). CID: /data/pixel-cast/cid (mount /data to keep the
# same CID across containers; without it, each container has its own).

FROM scratch AS minimal
ARG TARGETARCH
ENV HOME=/data XDG_CONFIG_HOME=/config XDG_DATA_HOME=/data
COPY docker/${TARGETARCH}/pixel-cast /usr/local/bin/pixel-cast
ENTRYPOINT ["/usr/local/bin/pixel-cast"]

FROM alpine:3.24 AS ffmpeg
RUN apk add --no-cache ffmpeg
ARG TARGETARCH
ENV HOME=/data XDG_CONFIG_HOME=/config XDG_DATA_HOME=/data
COPY docker/${TARGETARCH}/pixel-cast /usr/local/bin/pixel-cast
ENTRYPOINT ["/usr/local/bin/pixel-cast"]
