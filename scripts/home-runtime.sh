#!/bin/sh
# Installed beside bin/ and runtime/. Only /usr/lib is overlaid, read-only.
set -eu
mem_runtime_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mem_runtime_lib=$mem_runtime_dir/runtime/usr/lib
mem_runtime_bin=${MEM_INSPECTOR_BINARY:-$mem_runtime_dir/bin/mem-inspector-desktop}
if [ ! -x "$mem_runtime_bin" ] || [ ! -x "$mem_runtime_lib/webkit2gtk-4.1/WebKitWebProcess" ]; then
  printf '%s\n' 'Incomplete home-local Mem Inspector runtime.' >&2
  exit 1
fi
command -v bwrap >/dev/null || { printf '%s\n' 'bubblewrap is required for the user-space WebKit overlay.' >&2; exit 1; }
export LD_LIBRARY_PATH=$mem_runtime_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export WEBKIT_EXEC_PATH=$mem_runtime_lib/webkit2gtk-4.1
export WEBKIT_INJECTED_BUNDLE_PATH=$WEBKIT_EXEC_PATH/injected-bundle
export WEBKIT_INSPECTOR_RESOURCES_PATH=$mem_runtime_dir/runtime/usr/share/webkit2gtk-4.1/inspector
# Host access retains ordinary user permissions. This is a dependency overlay,
# not a security sandbox. WebKit's own sandbox remains enabled.
exec bwrap --bind / / --overlay-src /usr/lib --overlay-src "$mem_runtime_lib" --ro-overlay /usr/lib --proc /proc --dev-bind /dev /dev "$mem_runtime_bin" "$@"
