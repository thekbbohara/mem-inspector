#!/bin/sh
set -eu
mem_inspector_dir=${MEM_INSPECTOR_HOME:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
mem_inspector_runtime=${MEM_INSPECTOR_RUNTIME:-$HOME/.local/share/mem-inspector/current}
if [ -x "$mem_inspector_runtime/mem-inspector" ]; then
  exec "$mem_inspector_runtime/mem-inspector" "$@"
fi
mem_inspector_bin=${MEM_INSPECTOR_BINARY:-$mem_inspector_dir/target/release/mem-inspector-desktop}
if [ ! -x "$mem_inspector_bin" ]; then
  printf '%s\n' 'Mem Inspector native binary is missing. Run npm ci && npm run package from the source directory with Rust and Tauri system prerequisites installed.' >&2
  exit 1
fi
exec "$mem_inspector_bin" "$@"
