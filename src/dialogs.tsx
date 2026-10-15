import { useEffect, useRef, useState } from "react";
import { Play, Undo2, X } from "lucide-react";
import { api } from "./api";
import type { Card, Engine, Plan, Undo } from "./api";
import { middle, selectedBytes, size, tilde } from "./format";
import { Spinner } from "./ui";

function useEscape(onClose: () => void, blocked: boolean) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) =>
      e.key === "Escape" && !blocked && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, blocked]);
}

function Scrim({
  onClose,
  blocked,
  children,
}: {
  onClose: () => void;
  blocked: boolean;
  children: React.ReactNode;
}) {
  useEscape(onClose, blocked);
  return (
    <div
      className="scrim"
      onMouseDown={(e) => e.target === e.currentTarget && !blocked && onClose()}
    >
      {children}
    </div>
  );
}

/** Exact paths and sizes before a delete or move; live copy progress for moves. */
export function Preview({
  plan,
  card,
  home,
  onClose,
  onConfirm,
}: {
  plan: Plan;
  card: Card;
  home: string;
  onClose: () => void;
  onConfirm: (paths: string[]) => Promise<void>;
}) {
  const usable = plan.rows.filter((r) => r.ok !== false);
  const [picked, setPicked] = useState(
    () => new Set(usable.map((r) => r.path)),
  );
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  const move = plan.action === "move";
  useEffect(() => {
    if (!running || !move) return;
    const t = setInterval(async () => {
      const p = await api.progress().catch(() => null);
      if (p && p.total > 0) setProgress(p);
    }, 400);
    return () => clearInterval(t);
  }, [running, move]);
  const total = selectedBytes(plan.rows, picked);
  const all = picked.size === usable.length;
  const toggle = (path: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  return (
    <Scrim onClose={onClose} blocked={running}>
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="dlg-title"
      >
        <header>
          <div>
            <h2 id="dlg-title">{card.issue}</h2>
            <p className="quiet">
              {move
                ? `SSD to HDD (${middle(card.target ?? plan.target, 48)}). Copied, checked, linked back. Undo anytime.`
                : "Deleted for real, not moved to Trash."}
            </p>
          </div>
          <button
            className="icon-btn"
            onClick={onClose}
            disabled={running}
            aria-label="Close"
          >
            <X size={16} strokeWidth={1.75} />
          </button>
        </header>
        <div className="rows-head">
          <label className="check">
            <input
              type="checkbox"
              checked={all}
              disabled={running}
              onChange={() =>
                setPicked(all ? new Set() : new Set(usable.map((r) => r.path)))
              }
            />
            <span>
              {plan.rows.length} {plan.rows.length === 1 ? "item" : "items"}
            </span>
          </label>
        </div>
        <ul className="rows">
          {plan.rows.map((r) => (
            <li key={r.path} className={r.ok === false ? "row off" : "row"}>
              <label className="check">
                <input
                  type="checkbox"
                  disabled={r.ok === false || running}
                  checked={picked.has(r.path)}
                  onChange={() => toggle(r.path)}
                />
                <span className="path" title={r.path}>
                  {middle(tilde(r.path, home))}
                </span>
              </label>
              <span className="row-size">
                {r.ok === false ? "stays" : size(r.bytes)}
              </span>
            </li>
          ))}
        </ul>
        {running && move && (
          <div className="copy-progress">
            <div className="track">
              <span
                style={{
                  width: progress
                    ? `${(progress.done / Math.max(1, progress.total)) * 100}%`
                    : "2%",
                }}
              />
            </div>
            <span className="quiet tabular">
              {progress
                ? `Copying ${size(progress.done)} of ${size(progress.total)}`
                : "Checking"}
            </span>
          </div>
        )}
        <footer>
          <span className="quiet tabular">{size(total)} selected</span>
          <div className="actions">
            <button
              ref={cancel}
              className="btn ghost"
              onClick={onClose}
              disabled={running}
            >
              Cancel
            </button>
            <button
              className="btn primary"
              disabled={running || picked.size === 0}
              onClick={async () => {
                setRunning(true);
                try {
                  await onConfirm([...picked]);
                } finally {
                  setRunning(false);
                }
              }}
            >
              {running && <Spinner />}
              {move ? `Move ${size(total)}` : `Delete ${size(total)}`}
            </button>
          </div>
        </footer>
      </div>
    </Scrim>
  );
}

/** The confirm step for a command: the exact line that will run, nothing hidden. */
export function ConfirmRun({
  card,
  onClose,
  onRun,
}: {
  card: Card;
  onClose: () => void;
  onRun: () => Promise<void>;
}) {
  const [starting, setStarting] = useState(false);
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  return (
    <Scrim onClose={onClose} blocked={starting}>
      <div
        className="dialog narrow"
        role="dialog"
        aria-modal="true"
        aria-labelledby="run-title"
      >
        <header>
          <div>
            <h2 id="run-title">Run this command?</h2>
            <p className="quiet">
              {card.issue}. Frees about {size(card.bytes)}. Runs as you, never
              with sudo.
            </p>
          </div>
        </header>
        <pre className="run-line">
          <span className="prompt">$</span> {card.command}
        </pre>
        <footer>
          <span className="quiet">Output shows live below</span>
          <div className="actions">
            <button
              ref={cancel}
              className="btn ghost"
              onClick={onClose}
              disabled={starting}
            >
              Cancel
            </button>
            <button
              className="btn primary"
              disabled={starting}
              onClick={async () => {
                setStarting(true);
                try {
                  await onRun();
                } finally {
                  setStarting(false);
                }
              }}
            >
              {starting ? <Spinner /> : <Play size={13} strokeWidth={2} />}
              Run
            </button>
          </div>
        </footer>
      </div>
    </Scrim>
  );
}

export function Settings({
  engine,
  setEngine,
  undo,
  onUndo,
  onClose,
}: {
  engine: Engine;
  setEngine: (e: Engine) => void;
  undo: Undo[];
  onUndo: (id: string) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const away = (e: MouseEvent) =>
      !ref.current?.contains(e.target as Node) && onClose();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    const t = setTimeout(() => window.addEventListener("mousedown", away));
    window.addEventListener("keydown", esc);
    return () => {
      clearTimeout(t);
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", esc);
    };
  }, [onClose]);
  return (
    <div className="popover" ref={ref} role="dialog" aria-label="Settings">
      <div className="field">
        <span className="label">Engine</span>
        <div className="segmented">
          {["claude", "codex"].map((p) => (
            <button
              key={p}
              className={engine.provider === p ? "on" : ""}
              onClick={() => setEngine({ ...engine, provider: p })}
            >
              {p === "claude" ? "Claude" : "Codex"}
            </button>
          ))}
        </div>
      </div>
      <label className="field">
        <span className="label">Model</span>
        <input
          value={engine.model}
          placeholder="Default"
          spellCheck={false}
          onChange={(e) =>
            setEngine({ ...engine, model: e.target.value.trim() })
          }
        />
      </label>
      {undo.length > 0 && (
        <div className="field">
          <span className="label">On the HDD</span>
          {undo.slice(0, 4).map((u) => (
            <button
              key={u.id}
              className="btn ghost wide"
              onClick={() => onUndo(u.id)}
            >
              <Undo2 size={14} strokeWidth={1.75} />
              Move back {u.count} {u.count === 1 ? "item" : "items"} ·{" "}
              {size(u.bytes)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
