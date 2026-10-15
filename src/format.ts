const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Same rounding as the Rust side so cards and chat agree. */
export function size(bytes: number): string {
  let v = Math.max(0, bytes);
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 || v >= 10
    ? `${Math.round(v)} ${UNITS[i]}`
    : `${v.toFixed(1)} ${UNITS[i]}`;
}

export function tilde(path: string, home: string): string {
  if (!home) return path;
  if (path === home) return "~";
  return path.startsWith(home + "/") ? "~" + path.slice(home.length) : path;
}

export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function usedRatio(total: number, free: number): number {
  return total > 0 ? Math.min(1, Math.max(0, (total - free) / total)) : 0;
}

export function selectedBytes(
  rows: { path: string; bytes: number }[],
  picked: Set<string>,
) {
  return rows.reduce((sum, r) => (picked.has(r.path) ? sum + r.bytes : sum), 0);
}

/** Keep the start and the end of a long path, which carry the meaning. */
export function middle(path: string, max = 64): string {
  if (path.length <= max) return path;
  const keep = max - 1;
  const head = Math.ceil(keep * 0.4);
  return (
    path.slice(0, head) + "\u2026" + path.slice(path.length - (keep - head))
  );
}

/** "just now", "3 min ago", "2 h ago" for the last scan. */
export function ago(thenMs: number, nowMs: number): string {
  const s = Math.max(0, Math.round((nowMs - thenMs) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

/** Ease-out for count-ups: fast start, gentle landing. */
export function easeOut(t: number): number {
  const x = Math.min(1, Math.max(0, t));
  return 1 - Math.pow(1 - x, 3);
}

/** HDD folder for a card: the archive root and the last two folders, which carry the meaning. */
export function hddPath(target: string, archive: string): string {
  if (!archive || !target.startsWith(archive + "/")) return target;
  const rest = target.slice(archive.length + 1).split("/");
  return rest.length <= 2 ? target : `${archive}/…/${rest.slice(-2).join("/")}`;
}
