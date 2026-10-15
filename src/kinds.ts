/** Data types, in bar order. Colours live in app.css as --k-<key>. */
export const KINDS = [
  { key: "video", label: "Video" },
  { key: "images", label: "Images" },
  { key: "code", label: "Code" },
  { key: "deps", label: "Deps" },
  { key: "caches", label: "Caches" },
  { key: "models", label: "AI models" },
  { key: "archives", label: "Archives" },
  { key: "other", label: "Other" },
  { key: "system", label: "System" },
] as const;

export type KindKey = (typeof KINDS)[number]["key"];
export type Segment = {
  key: KindKey;
  label: string;
  bytes: number;
  share: number;
};

const LABEL = Object.fromEntries(KINDS.map((k) => [k.key, k.label])) as Record<
  KindKey,
  string
>;

export function kindLabel(key: string): string {
  return LABEL[key as KindKey] ?? "Other";
}

/**
 * Bar segments for one disk: every data type as a share of the whole disk
 * (used + free), in fixed order, tiny slivers dropped. Shares never exceed
 * the used part, so free space always shows.
 */
export function segments(
  kinds: Record<string, number> | null | undefined,
  used: number,
  free: number,
  minShare = 0.004,
): Segment[] {
  const whole = used + free;
  if (!kinds || whole <= 0) return [];
  const total = KINDS.reduce((s, k) => s + Math.max(0, kinds[k.key] ?? 0), 0);
  const scale = total > used && total > 0 ? used / total : 1;
  return KINDS.map((k) => {
    const bytes = Math.max(0, kinds[k.key] ?? 0) * scale;
    return { key: k.key, label: k.label, bytes, share: bytes / whole };
  }).filter((s) => s.share >= minShare);
}

/** Kinds that appear on any disk, for the legend. */
export function present(lists: Segment[][]): KindKey[] {
  const seen = new Set(lists.flat().map((s) => s.key));
  return KINDS.map((k) => k.key).filter((k) => seen.has(k));
}
