#!/bin/sh
# Run the built AppImage in a clean Ubuntu 24.04 container on a virtual
# display with a throwaway HOME, and screenshot the first-run flow.
# Usage: scripts/smoke-release.sh [APPIMAGE] [SHOTS_DIR]
set -eu
cd "$(dirname "$0")/.."
app=$(realpath "${1:-$(ls release/out/*.AppImage | head -1)}")
shots=$(realpath -m "${2:-release/out/smoke}")
mkdir -p "$shots"
image=mem-inspector-smoke:24.04
docker build -q -t "$image" release/smoke >/dev/null
docker run --rm --memory "${MEMORY_MAX:-3g}" \
  --tmpfs /second-disk:size=1g,mode=1777 \
  --user "$(id -u):$(id -g)" \
  -v "$app:/app/Mem.AppImage:ro" -v "$(pwd)/scripts:/scripts:ro" \
  -v "$(pwd)/release/smoke:/smoke:ro" -v "$shots:/shots" \
  "$image" sh /smoke/run.sh
ls -l "$shots"
