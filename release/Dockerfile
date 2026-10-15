# Release build image: Ubuntu 22.04 keeps the AppImage's glibc old enough to
# run on most current distros (Ubuntu 22.04+, Debian 12+, Fedora, Arch).
FROM ubuntu:22.04

ARG NODE_VERSION=22.12.0
ARG RUST_VERSION=1.99.1
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl wget file xz-utils git patchelf \
      pkg-config libssl-dev libwebkit2gtk-4.1-dev libxdo-dev \
      libayatana-appindicator3-dev librsvg2-dev python3 \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
      | tar -xJ -C /usr/local --strip-components=1

ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | CARGO_HOME=/opt/cargo sh -s -- -y \
      --profile minimal --default-toolchain "${RUST_VERSION}" \
    && chmod -R a+rX /opt/rustup /opt/cargo

# linuxdeploy is itself an AppImage; containers have no FUSE.
ENV APPIMAGE_EXTRACT_AND_RUN=1 NO_STRIP=1
WORKDIR /build
