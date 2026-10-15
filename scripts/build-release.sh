#!/bin/sh
# Build the Linux AppImage and .deb in a Docker container (no sudo, no system
# packages). Output: release/out/. Caches live in target/release-docker/.
# MEMORY_MAX (default 4g) caps the container; JOBS (default 1) sets cargo jobs.
set -eu
cd "$(dirname "$0")/.."
root=$(pwd)
image=mem-inspector-release:22.04
cache="$root/target/release-docker"
out="$root/release/out"
mkdir -p "$cache/cargo" "$cache/npm" "$cache/home" "$cache/src" "$out"

docker build -t "$image" release

# Fresh copy of the tracked (and new, not ignored) files: host node_modules,
# target and dist never leak into the release.
rm -rf "$cache/src"
mkdir -p "$cache/src"
git ls-files -co --exclude-standard -z | xargs -0 tar -cf - | tar -xf - -C "$cache/src"

docker run --rm \
  --memory "${MEMORY_MAX:-4g}" --memory-swap "${MEMORY_SWAP_MAX:-5g}" \
  --user "$(id -u):$(id -g)" \
  -e HOME=/cache/home -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm \
  -e CARGO_BUILD_JOBS="${JOBS:-1}" -e CARGO_TARGET_DIR=/cache/target \
  -v "$cache:/cache" -v "$out:/out" -w /cache/src \
  "$image" sh -euc '
    npm ci --no-audit --no-fund
    npx tauri build --bundles appimage,deb -- --locked
    cp /cache/target/release/bundle/appimage/*.AppImage /out/
    cp /cache/target/release/bundle/deb/*.deb /out/
  '
ls -lh "$out"
