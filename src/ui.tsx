import { useEffect, useRef, useState } from "react";
import {
  Archive,
  Brain,
  Code,
  Database,
  File,
  Film,
  Image,
  Package,
  SquareTerminal,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { Disk } from "./api";
import { easeOut, size } from "./format";
import { kindLabel, segments } from "./kinds";

const ICON: Record<string, LucideIcon> = {
  video: Film,
  images: Image,
  code: Code,
  deps: Package,
  caches: Database,
  models: Brain,
  archives: Archive,
  other: File,
  system: SquareTerminal,
};

const reduced = () =>
  typeof matchMedia === "function" &&
  matchMedia("(prefers-reduced-motion: reduce)").matches;

/** A number that eases from its last value to the new one. */
export function useCountUp(target: number, ms = 900): number {
  const [shown, setShown] = useState(target);
  const from = useRef(target);
  useEffect(() => {
    const start = from.current;
    if (start === target || reduced()) {
      from.current = target;
      setShown(target);
      return;
    }
    const t0 = performance.now();
    let frame = 0;
    const step = (now: number) => {
      const p = easeOut((now - t0) / ms);
      const v = start + (target - start) * p;
      from.current = v;
      setShown(v);
      if (p < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    // Land exactly on the value even when the window stops painting frames.
    const land = setTimeout(() => {
      cancelAnimationFrame(frame);
      from.current = target;
      setShown(target);
    }, ms + 60);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(land);
    };
  }, [target, ms]);
  return shown;
}

export function CountSize({ bytes, ms }: { bytes: number; ms?: number }) {
  return <>{size(useCountUp(bytes, ms))}</>;
}

export function Spinner() {
  return <span className="spinner" aria-hidden />;
}

export function Tile({ kind }: { kind: string }) {
  const Icon = ICON[kind] ?? File;
  return (
    <span className="tile" style={{ ["--k" as string]: `var(--k-${kind})` }}>
      <Icon size={14} strokeWidth={1.75} />
    </span>
  );
}

/** Used space split by data type; free space is the track. */
export function Bar({
  disk,
  big = false,
  focus,
}: {
  disk: Disk;
  big?: boolean;
  focus: string | null;
}) {
  // Grow from zero on first paint, then animate between values.
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const t = requestAnimationFrame(() => setReady(true));
    return () => cancelAnimationFrame(t);
  }, []);
  const segs = segments(disk.kinds, disk.used, disk.free);
  const whole = disk.used + disk.free;
  return (
    <div
      className={`bar${big ? " big" : ""}${disk.pending || !disk.kinds ? " measuring" : ""}`}
      role="img"
      aria-label={`${disk.name}: ${size(disk.used)} used, ${size(disk.free)} free`}
    >
      {segs.length > 0 ? (
        segs.map((s) => (
          <span
            key={s.key}
            className={focus && focus !== s.key ? "seg dim" : "seg"}
            title={`${s.label} · ${size(s.bytes)}`}
            style={{
              width: ready ? `${s.share * 100}%` : 0,
              background: `var(--k-${s.key})`,
            }}
          />
        ))
      ) : (
        <span
          className="seg plain"
          style={{
            width: ready && whole ? `${(disk.used / whole) * 100}%` : 0,
          }}
        />
      )}
    </div>
  );
}

export function Legend({
  keys,
  focus,
  setFocus,
}: {
  keys: string[];
  focus: string | null;
  setFocus: (k: string | null) => void;
}) {
  return (
    <div className="legend" onMouseLeave={() => setFocus(null)}>
      {keys.map((k) => (
        <span
          key={k}
          className={focus && focus !== k ? "dim" : ""}
          onMouseEnter={() => setFocus(k)}
        >
          <i className="sw" style={{ background: `var(--k-${k})` }} />
          {kindLabel(k)}
        </span>
      ))}
      <span>
        <i className="sw free" />
        Free
      </span>
    </div>
  );
}
