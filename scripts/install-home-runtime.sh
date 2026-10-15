#!/bin/sh
# Copy an already extracted Arch runtime prefix; never install system packages.
set -eu
mem_source_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mem_prefix=${1:?Usage: install-home-runtime.sh EXTRACTED_PREFIX [NEW_HOME_DESTINATION] [RELEASE_BINARY]}
mem_binary=${3:-$mem_source_dir/target/release/mem-inspector-desktop}
[ -x "$mem_binary" ] || { printf '%s\n' 'Build the release binary first.' >&2; exit 1; }
mem_hash=$(sha256sum "$mem_binary" | cut -d ' ' -f 1)
mem_destination=${2:-$HOME/.local/share/mem-inspector/t141-$mem_hash}
mem_destination=$(realpath -m -- "$mem_destination")
mem_real_home=$(realpath -- "$HOME")
case "$mem_destination" in "$mem_real_home"/.local/share/mem-inspector/*) ;; *) printf '%s\n' 'Destination must be a new directory under ~/.local/share/mem-inspector/.' >&2; exit 1;; esac
[ ! -e "$mem_destination" ] && [ ! -L "$mem_destination" ] || { printf '%s\n' 'Destination already exists; preserving it. Choose a new destination.' >&2; exit 1; }
for mem_required in libwebkit2gtk-4.1.so.0 libjavascriptcoregtk-4.1.so.0 webkit2gtk-4.1/WebKitWebProcess webkit2gtk-4.1/WebKitNetworkProcess; do
  [ -e "$mem_prefix/usr/lib/$mem_required" ] || { printf 'Missing runtime asset: %s\n' "$mem_required" >&2; exit 1; }
done
# A copied symlink must resolve inside the copied prefix, never to /tmp or
# another installation. Preserve relative links from trusted package extraction.
mem_prefix_root=$(realpath -- "$mem_prefix/usr")
find "$mem_prefix_root" -type l -print0 | xargs -0 -r sh -c '
  mem_expected=$1
  shift
  for mem_link do
    mem_target=$(realpath -e -- "$mem_link") || exit 1
    case "$mem_target" in "$mem_expected"/*) ;; *) printf "External runtime symlink: %s\n" "$mem_link" >&2; exit 1;; esac
  done
' sh "$mem_prefix_root"
umask 077
mkdir -p -- "$(dirname -- "$mem_destination")"
# Refuse a symlinked parent that would redirect installation outside the home.
mem_parent=$(realpath -- "$(dirname -- "$mem_destination")")
case "$mem_parent" in "$mem_real_home"/.local/share/mem-inspector|"$mem_real_home"/.local/share/mem-inspector/*) ;; *) exit 1;; esac
mkdir -- "$mem_destination"
mkdir -- "$mem_destination/bin" "$mem_destination/runtime"
cp -a -- "$mem_prefix/usr" "$mem_destination/runtime/"
cp -- "$mem_binary" "$mem_destination/bin/mem-inspector-desktop"
cp -- "$mem_source_dir/scripts/home-runtime.sh" "$mem_destination/mem-inspector"
chmod 700 "$mem_destination/mem-inspector" "$mem_destination/bin/mem-inspector-desktop"
(cd "$mem_destination" && find bin runtime -type f -exec sha256sum {} + > assets.sha256)
# Preserve any existing selection; each installation also has its exact command.
if [ ! -e "$mem_real_home/.local/share/mem-inspector/current" ] && [ ! -L "$mem_real_home/.local/share/mem-inspector/current" ]; then
  ln -s -- "$mem_destination" "$mem_real_home/.local/share/mem-inspector/current"
fi
printf '%s\n' "$mem_destination/mem-inspector"
