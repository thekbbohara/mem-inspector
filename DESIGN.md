# Mem Inspector - Design contract

Status: v3. The owner picked layout concept C, "Two lanes" (see Layout).

## Style

**Dark premium x Swiss utility, with life.** This is a desktop tool for one
technical owner who wants decisions, not reading. The references are Linear,
Raycast and Apple System Settings: quiet chrome, one accent, real type
hierarchy, hairlines in place of boxes. Dark premium fits because this is a
developer tool used next to a dark terminal and editor. Swiss utility adds the
discipline: a grid, aligned numbers, no ornament.

v2 was correct but read as plain. v3 adds life in three places only, each of
which carries information:

1. **Colour per data type.** Disk bars split used space into video, images,
   code, deps, caches, AI models, archives, other and system. The same colour
   marks a card's type tile, so a card visibly points at a part of the bar.
2. **Motion that confirms.** Bars grow to new values, new cards slide in, and a
   finished action counts up "Freed 12 GB". Motion only answers "did it work?"
   and "what changed?".
3. **Fast vs slow, said once.** Every disk carries its medium and speed: SSD
   "fast", HDD "slow, roomy". The placement advisor is built on that contrast.

Never use: purple or rainbow gradients, glassmorphism, emoji, badges,
drop-shadowed giant cards, centered hero text, filler copy. Text stays short:
one line for what, one for the fix, numbers over adjectives.

**Relaxed rule: data-type colour.** v2 said "no colored chips". v3 allows
colour for data types only: bar segments, an 8px legend swatch, and a 28px
tinted icon tile on cards (tile fill is the type colour at 14% alpha, icon at
full colour). Why: the owner asked for a richer disk picture, and colour is the
one channel that lets a glance link "the big yellow part of my SSD" to "the
card that frees it" without more words. Colour is never used for status,
emphasis or decoration, and never on text. The accent stays the only
action colour.

## Tokens

| Token | Value | Use |
|---|---|---|
| --bg | #09090b | window |
| --surface | #111114 | cards, composer, dialog, disk cards |
| --surface-2 | #18181c | hover, owner bubble, bar track (free space) |
| --line | #222227 | hairlines |
| --line-strong | #2e2e34 | focused input, dialog edge |
| --text | #eae7e2 | primary text |
| --text-2 | #9a9aa0 | secondary line (the fix) |
| --text-3 | #616167 | tertiary, labels, placeholders |
| --accent | #e8793a | the single accent: primary button, focus, full-disk warning |
| --ok | #7fb98f | only for the "Freed" check and its count-up |

Data types (bar order). Validated with the dataviz palette checker on
`--surface`: all inside the lightness band, chroma floor met, normal-vision
separation >= 19.7, contrast >= 3:1. The worst adjacent colour-blind pair
(models/caches, protan) is 6.5, legal only with secondary encoding, which v3
always has: 2px gaps between segments, a legend, segment tooltips, and an
icon on every card tile.

| Type | Colour | Card icon |
|---|---|---|
| video (incl. audio, renders) | #9085e9 | Film |
| images | #d55181 | Image |
| code (git repos) | #3987e5 | Code |
| deps (node_modules, venvs, SDKs, build output) | #c98500 | Package |
| caches | #199e70 | Database |
| AI models | #e66767 | Brain |
| archives (zip, tar, iso, backups) | #008300 | Archive |
| other | #5b5b63 | File |
| system (OS and apps outside home) | #34343b | none |
| free | --surface-2 track | none |

Type: Inter Variable (bundled, no network), 13px base, 1.5 line height.
Sizes 11 / 12 / 13 / 14 / 15, plus 18 for the result banner and 28 for the
free-space hero number. Weights 400 / 500 / 600. Sizes and
byte counts use tabular numerals. Paths use the system mono stack at 12px.
Tracking -0.01em on 14px and up.

Spacing 4 / 8 / 12 / 16 / 20 / 24 / 32. Radius 6px controls, 8px type tiles,
10px cards and dialog, 999px owner bubble. Borders are 1px hairlines. Shadow
only on the dialog and popover (one soft ambient layer), never on cards.

## Shared components

- **Disk card**: name (13px/600), medium line "SSD - fast" or "HDD - slow,
  roomy" (11px text-3, Zap or HardDrive icon), free space right (14px/600
  tabular) with "of 476 GB" under it. Bar 8px, radius 4, segments in type
  order with a 2px surface gap, free as the track. Hover on a segment shows
  "Deps - 111 GB". While a disk's types are still being measured the used part
  is one neutral fill with a slow shimmer; free space is always exact.
- **Legend**: one row under the disks, an 8px swatch and label per type that
  exists on any disk. Hover on a legend item dims every other type on every
  bar, so "where is my video" is one hover.
- **Suggestion card**: type tile left, issue (14px/500), fix (13px text-2),
  size right (15px/600 tabular) with Skip (ghost) and Do it under it. Only the
  first open card per lane gets the accent fill; the rest are solid grey, so
  one orange button per lane says "start here".
- **Command card (Run, not Copy)**: issue, then the exact command, read-only,
  in a mono chip, so the owner sees what will run. The button is Run (Play
  icon). Run opens a confirm dialog that shows the same exact line after a
  `$`, the expected size, and "Runs as you, never with sudo"; Cancel has
  focus. Confirm starts it in the terminal panel. A command that needs root
  is never run: the card says "Needs root. Run it in your own terminal" and
  its button is a quiet Root button (lock icon) that copies the line.
- **Placement card**: same frame. Line 2 is the route: "SSD -> HDD" then the
  mono target folder ("/media/disk/Archive/home/you/work", middle-truncated).
  Line 3 is the tradeoff in text-3 ("Loads slower; fine for footage you rarely
  open"). The button says Move. Under the placement cards, one muted tip says
  what stays on the SSD and why.
- **Preview dialog**: as v2 (560px, rows with checkboxes, Cancel focused). For
  moves the footer shows a thin progress bar and "Copying 3.2 of 12 GB" while
  the verified copy runs. Rows that cannot move show why ("in use").
- **Terminal panel**: slides up between the lanes and the chat dock (280px).
  Header: terminal icon, card title, the command in text-3 mono, status
  ("Running" with spinner, "Done - exit 0" in --ok, "Exit 1" in the models
  red), Stop while running, close (X) only after it ends. Body: xterm.js on
  #0b0b0d, 12px mono, ANSI colours mapped to the data-type palette, 5000 lines
  of scrollback, read-only. Output stays until the owner closes it.
- **Header**: logo and name left; right side "Scanned 3 min ago", a visible
  Rescan button (RotateCw, spins while scanning) and the gear. The gear keeps
  engine, model, the folder personal files go to (Change / Turn off) and
  moves to undo.
- **First run**: a narrow dialog over the app, not dismissable by Escape. Three
  lines (Clean, Move, You decide) with accent icons, then "Where should
  personal files go?" with the path field and a Choose folder button (native
  folder picker). Errors (same disk as home, not writable) show in --bad under
  the field. Footer: ghost "Skip, cleanup only" left, primary Continue right
  (enabled once a folder is saved). Without Claude Code, one quiet line says
  built-in rules rank the cleanup.
- **Moves off**: with no folder, the HDD lane shows one dashed box "Moving is
  off" with a Choose folder button instead of placement cards.
- **Logo**: "Link Back". A hollow rounded square (the slot that stays on the
  SSD, #eae7e2) joined by one bent link to a solid rounded square (the data
  now on the HDD, accent #e8793a). It is the app's own trick, the symlink,
  drawn left to right like the lanes. Header uses the small cut at 20px; app
  icons come from a 1024px tile (#111114, radius 22%) via `tauri icon`.
  Source tile: `src-tauri/icons/icon.png`.
- **Icons**: Lucide at 16px, 1.5 stroke; 14px inside type tiles.

## Motion

Base: 160-240ms, cubic-bezier(.16,1,.3,1). Nothing loops except spinners and
the measuring shimmer. Under prefers-reduced-motion every value jumps.

- Bars: segment widths transition over 700ms when values change; on first load
  they grow from 0, left to right.
- Cards: slide up 8px and fade in, staggered 50ms. A card that no longer
  applies fades and collapses its height (240ms). A size that changes counts to
  its new value.
- Freed moment: the check draws itself (stroke animation, 400ms), "Freed 12 GB"
  counts up over 900ms with ease-out, then the SSD free number on the disk card
  counts up and the card gets a 1.2s soft --ok ring. No confetti.
- Scanning: a live list of the biggest folders found so far, rows sliding in
  with growing bars, plus "212 GB measured" counting and a clock. The disk
  bars show the shimmer until types are known.

## Auto refresh and action results

After Do it, Move, Undo or a finished Run, without a full rescan:

1. The action returns live disk numbers at once. Bars and free space animate
   to them, and moved bytes shift from the SSD's type segment to the HDD's.
2. A result line appears where the action happened: check, "Freed 12 GB -
   SSD now 107 GB free" (moves: "Moved 6.2 GB to the HDD - SSD now 101 GB
   free"), with Undo for moves. The card itself collapses to that one line.
3. In the background, every remaining card is re-measured: paths that are
   gone or already moved drop out, sizes update with a count, and cards with
   nothing left collapse away. A muted line says "1 suggestion no longer
   applies" when that happens. Typical time: a few seconds; no AI call.
   Command cards are re-probed too (for example Docker's reclaimable bytes).
   A Run that exits 0 collapses its card to "Freed X" (measured from the
   disk's free space before and after); a non-zero exit keeps the card open
   and the output stays in the terminal.
4. The header's "Scanned X min ago" stays honest: a recheck is not a scan.
   Rescan is always one click in the header.

## Placement advisor (SSD -> HDD)

Section title "Put data on the right disk", subtitle "Fast SSD -> slow,
roomy HDD. Links keep every path working." Cards, biggest first, one per
group: large videos and media, Downloads older than 30 days, idle git projects
(no commits or edits in 60+ days), AI models not loaded in 14+ days, Android
system images, archives, backups and disk images, big files untouched 90+
days. Each card: what, size, "SSD -> HDD (<archive folder>/...)", one-line
tradeoff. Moves copy, compare file count and bytes, park the original, link
back, and can be undone. Never offered: dotfiles and config, tracked files or
git data of repos, active projects, anything open in a running app, anything
owned by another user. The tip line: "Keep on the SSD: the OS, active
projects, node_modules, caches, browser profiles, Docker and models you use
daily. They need fast random reads."

## Layout: two lanes (concept C, chosen)

Header (logo, scanned label, Rescan, gear), then two lanes, then the chat dock.

- **SSD lane (left, 1.25fr)**: "SYSTEM SSD fast - 476 GB"; the free space as
  the one hero number (28px/600, counts up after actions) with "80% full" or
  "+18 GB this session" under it and the top three data types to the right;
  the 14px type-split bar; the legend; the latest result banner; then "FREE UP
  SPACE" with the cleanup and command cards.
- **HDD lane (right, 1fr, #0c0c0f)**: "HDD slow, roomy - 779 GB free"; one
  compact card per other disk with its bar; then "BELONGS ON THE HDD" with the
  placement cards (dashed border, "on SSD now" tag, arrow to the HDD folder,
  tradeoff, Move ->) and the keep-on-SSD tip.
- **Chat dock (bottom)**: the last question and answer on the left, the
  composer on the right. A suggestion from chat lands at the top of the lane
  it belongs to.
- **Terminal**: when a command runs, the panel opens between lanes and dock.

Other concepts considered: A, a refined single column, and B, a storage map
with a side pane.

## Responsive

Minimum window is 760x540. Below 900px the lanes stack, SSD first, and scroll
as one; the dock stacks the last answer over the composer. Browser preview (no native bridge) shows one
muted line saying so.

## Copy

Plain, short, no em dash. One line for the issue, one line for the fix, one
for a tradeoff. Claude answers in at most three short lines.
