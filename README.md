# Mem Inspector

A Linux desktop app that frees space on a nearly full SSD. It scans your home
folder, shows what is taking space by type (video, images, code, dependencies,
caches, AI models, archives), and turns that into short cards you can act on:
delete rebuildable caches, move cold data to a roomy HDD, or run a known cleanup
command. You decide every action: **Do it**, **Move**, **Run** or **Skip**.

Built with [Tauri 2](https://tauri.app) and Rust on the backend and React +
TypeScript on the frontend. Linux only.

> Screenshots: coming soon. Any screenshot added here must come from the
> rehearsal home (`scripts/rehearsal-home.py`), never from a real home folder.

## Features

- **Two lanes.** The fast SSD (what to clean) on the left, the slow, roomy HDD
  (what belongs there) on the right. Every disk bar is split by data type.
- **Cleanup cards.** Known caches, `node_modules` / `.venv` / `target` in
  projects idle 30+ days, Docker build cache and unused images, uv and Go
  caches. Sizes are the bytes really freed (hardlinks are counted correctly).
- **Placement advisor (SSD to HDD).** Large videos and media, old Downloads,
  git projects idle 60+ days, AI models unused for 14+ days, Android system
  images, archives, disk images and big files untouched for 90+ days. Each card
  shows the target folder and a one-line tradeoff. No AI needed.
- **Optional AI ranking.** If the Claude Code or Codex CLI is installed and
  logged in, it ranks the cards and answers questions such as "what should I
  move to the HDD?". It runs tool-free; every card it returns is checked
  against the scan facts, and invented paths or commands are dropped. Without
  it, built-in rules cover everything.
- **Built-in terminal.** Cleanup commands run in a PTY and stream into an
  xterm.js panel with live output, exit code and scrollback.
- **Undo for moves**, which survives restarts.
- **Recheck, not rescan.** After every action the remaining cards are
  rechecked in seconds; a full rescan is one click.

## Safety rules

- **Home only.** Nothing outside your home folder is touched, except the HDD
  archive target. Nothing reached through a symlink is touched.
- **No sudo, ever.** Nothing runs with sudo, doas, pkexec, su or run0. Commands
  that need root are shown with "Needs root" and a copy button for you to run.
- **Personal files are never deleted.** Delete only applies to rebuildable data:
  paths inside known cache roots or folders named `node_modules`, `.venv`,
  `venv`, `__pycache__` or `target` (next to a `Cargo.toml`). Git repositories
  and `.git` never qualify.
- **Moves keep a link back.** A moved file or folder is copied to the archive,
  verified (file, folder and link counts and bytes, after a `syncfs`), and only
  then replaced by a symlink to its new place, so every path keeps working.
  Each move is logged before the source goes away and can be undone.
- **Moves refuse** hidden files and config, cache content, files tracked by
  git, recently changed projects and folders, anything open in a running
  process, anything owned by another user, special files, nested mounts, and
  moves that would leave the HDD with less than 1 GB free.
- **Never touched:** `~/.ssh`, `~/.gnupg`, keyrings, browser profiles, AI tool
  config folders and other credential folders.
- **Run needs a confirm.** The UI sends a card id, never a command line; the
  command comes from a fixed list in `src-tauri/src/tools.rs` and the backend
  refuses without confirmation.
- Every path is rechecked right before it is touched.

## Build and run (Linux)

Requirements: Node 22+, Rust stable, and the
[Tauri Linux prerequisites](https://tauri.app/start/prerequisites/#linux)
(GTK3, WebKitGTK 4.1, libsoup3, a C compiler).

```sh
git clone https://github.com/thekbbohara/mem-inspector.git
cd mem-inspector
npm ci
npm run desktop        # development: Vite + the native app
npm run package        # release binary and OS packages
```

Standalone release binary without OS packages:

```sh
npm run build
cargo build --release --locked --features custom-protocol
sh scripts/mem-inspector.sh
```

The `custom-protocol` feature is required for release builds; it embeds the
frontend so no HTTP listener is needed.

### Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `MEM_INSPECTOR_ARCHIVE` | `/mnt/data2/archive` | HDD folder that moves go to |
| `MEM_INSPECTOR_SCAN_HOME` | `$HOME` | Home folder to scan (rehearsals) |
| `MEM_INSPECTOR_BINARY` | `target/release/mem-inspector-desktop` | Binary the launcher runs |

State lives in `~/.local/state/mem-inspector/` (`moves.jsonl`, disk type cache).

### Try it on fake data

```sh
python3 scripts/rehearsal-home.py /var/tmp/mi-demo /path/on/hdd/mi-archive
MEM_INSPECTOR_SCAN_HOME=/var/tmp/mi-demo/home \
MEM_INSPECTOR_ARCHIVE=/path/on/hdd/mi-archive npm run desktop
```

This builds a disposable home with caches, dependencies, videos and old
downloads so you can try delete, move, run and undo end to end.

### Checks

```sh
npm test && npm run build
sh scripts/check-native.sh   # fmt, tests, clippy -D warnings, build for all crates
```

## Stack and layout

| Path | What |
| --- | --- |
| `src/` | React 19 + TypeScript UI (Vite, lucide icons, xterm.js) |
| `src-tauri/src/bridge.rs` | Tauri commands between UI and backend |
| `src-tauri/src/advisor.rs` | Turns scan results into cleanup facts and cards |
| `src-tauri/src/placement.rs` | SSD to HDD placement advisor |
| `src-tauri/src/space.rs` | Delete, move, undo, with all safety checks |
| `src-tauri/src/tools.rs` | Fixed list of cleanup commands, PTY runner |
| `scripts/storage-overview.py` | One `du` pass over home, grouped by data type |
| `crates/mem-core` | Bounded filesystem inspection core |
| `crates/mem-ai` | Isolated, tool-free CLI adapters for AI engines |
| `DESIGN.md` | UI design contract |

## License

[MIT](LICENSE) - Kamal Bohara (thekbbohara), 2026.
