# Mem Inspector

See what fills your Linux disk and free space safely. Mem Inspector scans your
home folder, then shows two lists: things to **clean** (caches and build
folders that rebuild themselves) and big personal files to **move** to a second
disk (videos, old downloads, idle projects, AI models). You decide on every
card: Do it, Move, Run or Skip.

## Download and run

1. Download `Mem.Inspector_<version>_amd64.AppImage` from the Releases page.
2. Make it executable and run it:

   ```sh
   chmod +x Mem.Inspector_*_amd64.AppImage
   ./Mem.Inspector_*_amd64.AppImage
   ```

On first launch it asks where personal files should go. Pick a folder on a
second disk (an HDD or external drive), or choose "Skip, cleanup only". You can
change this any time in Settings (gear icon). The choice is saved in
`~/.config/mem-inspector/settings.json`.

Claude Code is optional. If it is installed, it ranks the cleanup cards (it
uses `$CLAUDE_CONFIG_DIR`, else `~/.claude`). Without it, built-in rules do
the ranking and everything else works the same.

A `.deb` package is also published for Debian and Ubuntu:
`sudo apt install ./mem-inspector_<version>_amd64.deb`.

## Safety rules

- **Preview first.** Every action opens a list of the exact files and sizes
  before anything happens. Nothing runs until you confirm.
- **Personal files are moved, never deleted.** They are copied to your chosen
  folder, checked (file count and bytes), and a link is left behind, so every
  path still works. Undo moves them back.
- **Only rebuildable data is deleted**: known caches and dependency folders
  such as `node_modules`, `.venv` and Rust `target`.
- **Never touched**: anything outside your home, `~/.ssh`, `~/.gnupg`,
  keyrings, browser profiles, Claude and Codex settings, other credential
  folders, files open in a running app, and anything that needs root. Commands
  that need root are shown for you to copy, never run.
- No folder chosen means moves are off; cleanup still works.

To protect more folders, add them (relative to home) to `settings.json`:
`"protect": ["Work/clients", ".my-agent"]`.

## For developers

Install Node 22+, Rust stable, and Tauri's Linux prerequisites including GTK3, WebKitGTK 4.1, JavaScriptCoreGTK 4.1, libsoup3 and a C compiler. No privileged installation is performed by the app.

```sh
npm ci
npm run desktop
# Compile the frontend and native release binary/package:
npm run package
# Build a standalone release without creating OS packages (build frontend first):
npm run build
cargo build --release --locked --features custom-protocol
# Launch an existing release binary:
sh scripts/mem-inspector.sh
```

Standalone releases embed `dist/ui` and use Tauri's local asset protocol; they
need no HTTP listener. The `custom-protocol` feature is required even with
`--release`: without it Tauri uses the development URL `http://127.0.0.1:1420`.
The release build guard rejects that configuration. Development remains
`npm run desktop`, which starts Vite and the native app together. Verify the
production embedding with `cargo test --locked --features custom-protocol` after
building the frontend.

The launcher locates its repository relative to the script. MEM_INSPECTOR_HOME changes the source location; MEM_INSPECTOR_BINARY selects a prebuilt binary. Standard CARGO_HOME/RUSTUP_HOME/PATH and PKG_CONFIG_PATH/LD_LIBRARY_PATH support user-space toolchains and native dependency prefixes. `npm run dev` alone serves a browser preview that says to open the desktop app; it cannot touch files. The desktop has a root Rust workspace; both sibling crates remain standalone. Run `sh scripts/check-native.sh` for fmt, tests, Clippy with warnings denied and build across all three manifests. Root `cargo test --workspace` covers the host bridge; standalone crate tests are also run by that script. Frontend checks: npm test and npm run build.

### How it works

1. On launch, `scripts/storage-overview.py` runs one `du -a --time` pass over
   home (never follows symlinks). It yields folder totals, used space by data
   type (video, images, code, deps, caches, AI models, archives, other), big
   files, git repos with their newest change, and the entries of Downloads.
   After it, the other disks are measured the same way in the background and
   cached in `~/.local/state/mem-inspector/disk-kinds.json`, so every bar is
   split by type. The SSD's remainder is shown as System.
2. `src-tauri/src/advisor.rs` turns the scan into facts: known caches,
   `node_modules`/`.venv`/`target` in projects idle 30+ days, and cleanup
   commands from `src-tauri/src/tools.rs` (Docker build cache and unused
   images, uv and Go caches; pacman and journal need root). Sizes are the bytes
   really freed: a hardlinked inode only counts when every link is selected.
3. `src-tauri/src/placement.rs` finds data on the SSD that belongs on the HDD:
   large videos and media, Downloads older than 30 days, git projects idle 60+
   days, AI models not loaded in 14+ days, Android system images, archives,
   backups and disk images, and big files untouched 90+ days. Each card shows
   the HDD folder and a one-line tradeoff. These cards need no AI.
4. Claude (CLI adapter in `crates/mem-ai`, tool-free, config dir
   `$CLAUDE_CONFIG_DIR` or `~/.claude`) gets the facts and returns cleanup cards; Codex is the
   fallback; built-in rules cover the rest. Every card is checked against the
   facts: unknown paths, mismatched actions and invented commands are dropped,
   and sizes always come from the facts. Chat answers "what should I move to
   the HDD?" from the placement facts.
5. After any action the app rechecks the remaining cards (`recheck`): paths
   that are gone or moved drop out, sizes and command probes update. No rescan.
   Rescan is a button in the header.

### Actions and safety

`src-tauri/src/space.rs` rechecks every path right before it is touched.

- **Delete** (caches and dependencies): removed for real, not to Trash, with
  std's symlink-safe `remove_dir_all`. Only paths inside the known cache roots
  or named `node_modules`/`.venv`/`venv`/`__pycache__`/`target` (next to a
  `Cargo.toml`) qualify. Git repositories and `.git` never do.
- **Move** (placement): files and whole folders go to the folder picked on
  first run + the original absolute path. Folders are copied (symlinks kept as symlinks),
  flushed with one `syncfs`, and compared by file, folder and link count and
  bytes; files by size. Only then is the original renamed aside, the symlink
  created, and the original removed. Each move is logged before the source
  disappears, in `~/.local/state/mem-inspector/moves.jsonl`; undo copies back
  the same verified way, survives restarts and refuses when the link changed.
  Moves refuse: hidden files and config (except model stores under
  `~/.ollama/models`), cache content (except Android system images), files
  tracked by git (git-ignored files in a repo may move), git projects changed
  in the last 60 days, folders changed in the last 14 days, anything open in a
  running process (open files, mapped files and working directories read from
  `/proc`, as lsof does), anything owned by another user, special files,
  nested mounts, and moves that would leave the HDD with less than 1 GB.
- **Run** (commands): the UI sends a card id, never a command line; the
  command comes from the fixed list in `tools.rs`, built for the scanned home.
  Run shows the exact line and needs a confirm; the backend refuses without
  it. It runs as you in a PTY (portable-pty) and streams to an xterm.js panel
  with live output, exit code and scrollback. Nothing is ever run with sudo,
  doas, pkexec, su or run0: commands that need root are shown with "Needs
  root" and a copy button. A rehearsal home only gets home-scoped commands.
- Never touched: anything outside home (except the archive target and the
  commands above), anything reached through a symlink, `~/.ssh`, `~/.gnupg`,
  keyrings, browser profiles, the `protect` list in settings, `~/.claude*`,
  `~/.codex*`, other credential folders.

Overrides for rehearsal and tests (users never need them):
`MEM_INSPECTOR_SCAN_HOME` and `MEM_INSPECTOR_ARCHIVE`. Build a disposable fake
home with `python3 scripts/rehearsal-home.py DIR [ARCHIVE]` and launch against
it. Put DIR on the SSD (for example under /var/tmp) and ARCHIVE on the HDD to
see real SSD to HDD moves. Never point a rehearsal at real data.

To preview suggestions on a real scan without touching anything:
`MEM_OVERVIEW_JSON=scan.json cargo test real_home -- --ignored --nocapture`.

### Home-local runtime on Arch without WebKitGTK

An Arch workstation can launch without a system WebKitGTK 4.1 install. The home-local runtime uses host GTK3, libsoup3, graphics libraries, and bubblewrap with unprivileged user namespaces/read-only overlay support. It bundles the extracted Arch WebKitGTK 4.1/JavaScriptCore shared libraries, WebKit subprocesses/resources, harfbuzz-icu, libmanette, enchant and hyphen. It is an Arch x86_64 dependency overlay, not a portable Linux package or security sandbox. WebKit's own sandbox remains enabled. No Rust, Cargo, Node, or source checkout is needed after installation.

Build first with a compatible development prefix, then copy that extracted prefix and the optimized binary:

```sh
sh scripts/install-home-runtime.sh /absolute/path/to/extracted-arch-prefix
# The installer prints the exact persistent executable command.
~/.local/share/mem-inspector/current/mem-inspector
# From this checkout, the same selected installation is used:
sh scripts/mem-inspector.sh
```

The prefix must contain `usr/lib` and the WebKit subprocesses; it can be prepared with user-space `tar --zstd -xf package.pkg.tar.zst -C PREFIX` extraction of trusted Arch packages. Never use a system package installation command for this setup. The installer performs no downloads and preserves existing directories and the existing `current` selection. An explicit second argument chooses a new destination under `~/.local/share/mem-inspector/`; the optional third argument chooses the release binary. Default installations are named by the binary SHA256. To select a different existing runtime without changing files, use `MEM_INSPECTOR_RUNTIME=/absolute/home/runtime sh scripts/mem-inspector.sh`. `MEM_INSPECTOR_BINARY` still selects a different binary with the selected runtime libraries. The installed launcher derives asset paths from itself and internally configures its library/subprocess paths; do not source a temporary toolchain environment to launch it.

Only `/usr/lib` is overlaid read-only. Other host paths retain ordinary user permissions so native scans and OS Trash work. Installation assets remain under the user's home. Host ABI compatibility and bubblewrap support are required; the copied `assets.sha256` records file integrity, not package authenticity. Choose a trusted matching package source. Existing assets are never replaced, and a failed partial installation is retained for inspection rather than removed automatically.
