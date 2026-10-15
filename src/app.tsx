import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  ArrowRight,
  ArrowUp,
  Check,
  HardDrive,
  Lock,
  Play,
  RotateCw,
  Settings2,
  Undo2,
  Zap,
} from "lucide-react";
import "@fontsource-variable/inter";
import "./app.css";
import { api } from "./api";
import type {
  Card,
  Disk,
  Engine,
  Folder,
  Outcome,
  Plan,
  Turn,
  Undo,
} from "./api";
import { ago, clock, hddPath, middle, size, tilde, usedRatio } from "./format";
import { present, segments } from "./kinds";
import { Bar, CountSize, Legend, Spinner, Tile, useCountUp } from "./ui";
import { ConfirmRun, Preview, Settings } from "./dialogs";
import { TerminalPanel, terminalSize } from "./terminal";
import type { RunState } from "./terminal";

type CardState = "open" | "skipped" | "done" | "undone" | "ran" | "gone";
type Slot = { card: Card; state: CardState; outcome?: Outcome; note?: string };
type Result = {
  key: number;
  title: string;
  bytes: number;
  verb: string;
  free: number;
  undo?: string | null;
  card?: string;
};
type Busy = { label: string; since: number } | null;

const ENGINE_KEY = "mem-inspector.engine";
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

function loadEngine(): Engine {
  try {
    const saved = JSON.parse(localStorage.getItem(ENGINE_KEY) ?? "null");
    if (saved && typeof saved.provider === "string")
      return { model: "", ...saved };
  } catch {
    /* defaults below */
  }
  return { provider: "claude", model: "" };
}

function Logo() {
  // The "Link Back" mark, small cut: a hollow slot stays, a link leads to the data.
  return (
    <svg className="logo" viewBox="0 0 256 256" aria-hidden>
      <path
        fill="#eae7e2"
        d="M40 16H88A24 24 0 0 1 112 40V88A24 24 0 0 1 88 112H40A24 24 0 0 1 16 88V40A24 24 0 0 1 40 16ZM52 48A4 4 0 0 0 48 52V76A4 4 0 0 0 52 80H76A4 4 0 0 0 80 76V52A4 4 0 0 0 76 48Z M108 48H160A48 48 0 0 1 208 96V148H176V96A16 16 0 0 0 160 80H108Z"
      />
      <path
        fill="#e8793a"
        d="M168 144H216A24 24 0 0 1 240 168V216A24 24 0 0 1 216 240H168A24 24 0 0 1 144 216V168A24 24 0 0 1 168 144Z"
      />
    </svg>
  );
}

function CardView({
  slot,
  home,
  archive,
  index,
  primary,
  loading,
  onDo,
  onSkip,
  onUndo,
}: {
  slot: Slot;
  home: string;
  archive: string;
  index: number;
  primary: boolean;
  loading: boolean;
  onDo: () => void;
  onSkip: () => void;
  onUndo: () => void;
}) {
  const { card, state, outcome } = slot;
  const [copied, setCopied] = useState(false);
  const style = { animationDelay: `${Math.min(index, 8) * 50}ms` };
  if (state === "gone")
    return (
      <article className="card gone" style={style}>
        <Tile kind={card.kind} />
        <p className="issue">{card.issue}</p>
        <span className="quiet small">No longer applies</span>
      </article>
    );
  if (state !== "open") {
    const ok = state === "done" || state === "ran";
    const label =
      state === "skipped"
        ? "Skipped"
        : state === "undone"
          ? "Moved back"
          : state === "ran"
            ? (slot.note ?? "Ran")
            : outcome
              ? `${card.action === "move" ? "Moved" : "Freed"} ${size(outcome.freed)}${
                  outcome.failed.length
                    ? `, ${outcome.failed.length} stayed`
                    : ""
                }`
              : "";
    return (
      <article className="card resolved" style={style}>
        <Tile kind={card.kind} />
        <div className="main">
          <p className="issue">{card.issue}</p>
          {card.action === "move" && state === "done" && card.target && (
            <p className="target mono">{hddPath(card.target, archive)}</p>
          )}
        </div>
        <span className="resolved-right">
          <span className={ok ? "outcome ok" : "outcome"}>
            {ok && <Check size={13} strokeWidth={2.25} />}
            {label}
          </span>
          {state === "done" && outcome?.undo && (
            <button className="btn ghost" onClick={onUndo}>
              <Undo2 size={13} strokeWidth={1.75} />
              Undo
            </button>
          )}
        </span>
      </article>
    );
  }
  const move = card.action === "move";
  const command = card.action === "command";
  return (
    <article className={move ? "card incoming" : "card"} style={style}>
      <Tile kind={card.kind} />
      <div className="main">
        {move && (
          <span className="from">
            <Zap size={11} strokeWidth={2} />
            on SSD now
          </span>
        )}
        <p
          className="issue"
          title={card.paths.map((p) => tilde(p, home)).join("\n")}
        >
          {card.issue}
        </p>
        {move ? (
          <>
            <p className="target mono" title={card.target}>
              <ArrowRight size={11} strokeWidth={2} />
              {hddPath(card.target ?? "", archive)}
            </p>
            <p className="trade">{card.tradeoff ?? card.fix}</p>
          </>
        ) : command ? (
          <>
            <code className="cmd" title={card.command ?? ""}>
              {card.command}
            </code>
            {card.root && (
              <p className="trade">Needs root. Run it in your own terminal.</p>
            )}
          </>
        ) : (
          <p className="fix">{card.fix}</p>
        )}
      </div>
      <div className="side">
        <span className="size tabular">
          <CountSize bytes={card.bytes} ms={600} />
        </span>
        <span className="actions">
          <button className="btn ghost" onClick={onSkip}>
            Skip
          </button>
          {command && card.root ? (
            <button
              className="btn solid"
              title="Copy the command"
              onClick={async () => {
                await navigator.clipboard.writeText(card.command ?? "");
                setCopied(true);
                setTimeout(() => setCopied(false), 1600);
              }}
            >
              {copied ? (
                <Check size={13} strokeWidth={2} />
              ) : (
                <Lock size={13} strokeWidth={1.75} />
              )}
              {copied ? "Copied" : "Root"}
            </button>
          ) : (
            <button
              className={primary ? "btn primary" : "btn solid"}
              onClick={onDo}
              disabled={loading}
            >
              {loading ? (
                <Spinner />
              ) : command ? (
                <Play size={12} strokeWidth={2} />
              ) : null}
              {command ? "Run" : move ? "Move" : "Do it"}
              {move && !loading && <ArrowRight size={13} strokeWidth={2} />}
            </button>
          )}
        </span>
      </div>
    </article>
  );
}

function Skeleton({ n, label }: { n: number; label: string }) {
  return (
    <>
      {Array.from({ length: n }, (_, i) => (
        <div
          key={i}
          className="card skeleton"
          style={{ animationDelay: `${i * 80}ms` }}
        >
          <span className="tile" />
          <div className="main">
            <span className="line w60" />
            <span className="line w40" />
          </div>
        </div>
      ))}
      <p className="hint status">
        <Spinner />
        {label}
      </p>
    </>
  );
}

function Scanning({
  folders,
  measured,
  since,
  now,
}: {
  folders: Folder[];
  measured: number;
  since: number;
  now: number;
}) {
  const top = folders.slice(0, 7);
  const max = Math.max(1, ...top.map((f) => Number(f.allocated)));
  const shown = useCountUp(measured, 900);
  return (
    <div className="scanning">
      <div className="scan-head">
        <Spinner />
        <span>Measuring your home</span>
        <span className="quiet tabular">{clock((now - since) / 1000)}</span>
        <span className="scan-total tabular">{size(shown)}</span>
      </div>
      <ul className="scan-list">
        {top.map((f) => (
          <li key={f.path}>
            <span className="mono">
              {middle(f.path.split("/").pop() ?? f.path, 28)}
            </span>
            <span className="scan-bar">
              <i style={{ width: `${(Number(f.allocated) / max) * 100}%` }} />
            </span>
            <span className="tabular quiet">{size(Number(f.allocated))}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ResultBanner({
  result,
  onUndo,
}: {
  result: Result;
  onUndo: () => void;
}) {
  const freed = useCountUp(result.bytes, 900);
  const free = useCountUp(result.free, 1100);
  return (
    <div className="banner" key={result.key}>
      <span className="freed-mark">
        <svg viewBox="0 0 24 24">
          <path d="M20 6 9 17l-5-5" />
        </svg>
      </span>
      <div>
        <b className="ok tabular">
          {result.verb} {size(freed)}
          {result.title}
        </b>
        <span className="sub tabular">SSD now {size(free)} free</span>
      </div>
      {result.undo && (
        <button className="btn solid" onClick={onUndo}>
          <Undo2 size={13} strokeWidth={1.75} />
          Undo
        </button>
      )}
    </div>
  );
}

function App() {
  const [disks, setDisks] = useState<Disk[]>([]);
  const [home, setHome] = useState("");
  const [archive, setArchive] = useState("");
  const [scannedAt, setScannedAt] = useState<number | null>(null);
  const [cleanup, setCleanup] = useState<Slot[] | null>(null);
  const [placement, setPlacement] = useState<Slot[] | null>(null);
  const [keep, setKeep] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState<Busy>(null);
  const [scan, setScan] = useState<{
    folders: Folder[];
    measured: number;
  } | null>(null);
  const [now, setNow] = useState(Date.now());
  const [draft, setDraft] = useState("");
  const [chat, setChat] = useState<{ q: string; a: string | null } | null>(
    null,
  );
  const [history, setHistory] = useState<Turn[]>([]);
  const [engine, setEngineState] = useState(loadEngine);
  const [gear, setGear] = useState(false);
  const [undo, setUndo] = useState<Undo[]>([]);
  const [preview, setPreview] = useState<{ plan: Plan; card: Card } | null>(
    null,
  );
  const [confirmRun, setConfirmRun] = useState<Card | null>(null);
  const [term, setTerm] = useState<RunState | null>(null);
  const [planning, setPlanning] = useState<string | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [gained, setGained] = useState(0);
  const [focus, setFocus] = useState<string | null>(null);
  const [glow, setGlow] = useState(false);
  const run = useRef(0);
  const resultKey = useRef(0);

  const setEngine = (e: Engine) => {
    setEngineState(e);
    try {
      localStorage.setItem(ENGINE_KEY, JSON.stringify(e));
    } catch {
      /* per-viewer convenience only */
    }
  };

  const refreshDisks = useCallback(async () => {
    const d = await api.disks();
    setDisks(d.disks);
    setHome(d.home);
    setArchive(d.archive);
    setUndo(d.undo);
    setScannedAt(d.scannedAt ? d.scannedAt * 1000 : null);
    return d.disks;
  }, []);

  const patch = (id: string, change: Partial<Slot>) => {
    const apply = (list: Slot[] | null) =>
      list && list.map((s) => (s.card.id === id ? { ...s, ...change } : s));
    setCleanup(apply);
    setPlacement(apply);
  };

  const start = useCallback(
    async (fresh: boolean) => {
      const token = ++run.current;
      setCleanup(null);
      setPlacement(null);
      setResult(null);
      setNote(null);
      setScan({ folders: [], measured: 0 });
      setBusy({ label: "Measuring", since: Date.now() });
      try {
        await api.overview(fresh);
        for (;;) {
          const o = await api.overview();
          await refreshDisks().catch(() => undefined);
          if (token !== run.current) return;
          const ov = o.overview;
          setScan({
            folders: ov?.folders ?? [],
            measured: Number(ov?.allocated ?? 0),
          });
          if (!o.pending && ov?.complete) break;
          await new Promise((r) => setTimeout(r, 1000));
        }
        setScan(null);
        setBusy(null);
        // Placement is rule-based and fast; cleanup waits for the AI.
        const places = api.placement().then((p) => {
          if (token !== run.current) return;
          setKeep(p.keep);
          setPlacement(p.cards.map((card) => ({ card, state: "open" })));
        });
        const cleans = api.suggest(engine).then((s) => {
          if (token !== run.current) return;
          setHome(s.home);
          setCleanup(s.suggestions.map((card) => ({ card, state: "open" })));
          if (s.source === "rules")
            setNote("AI is unavailable, so these come from built-in rules.");
          else if (s.source !== "claude")
            setNote(`Claude is unavailable, ${s.source} answered.`);
        });
        await Promise.allSettled([places, cleans]);
        await places.catch(
          (e) => token === run.current && setNote(errorText(e)),
        );
        await cleans.catch((e) => {
          if (token === run.current) {
            setCleanup([]);
            setNote(errorText(e));
          }
        });
      } catch (e) {
        if (token === run.current) {
          setScan(null);
          setNote(errorText(e));
        }
      } finally {
        if (token === run.current) setBusy(null);
      }
    },
    // The engine is read once per scan; changing it applies to the next request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [refreshDisks],
  );

  useEffect(() => {
    start(false);
  }, [start]);

  // Clock for "scanned X ago" and the scan timer.
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), busy ? 1000 : 20000);
    return () => clearInterval(t);
  }, [busy]);

  // HDD types arrive in the background; poll until they do, then rarely.
  const pending = disks.some((d) => d.pending);
  useEffect(() => {
    if (busy) return;
    const t = setInterval(
      () => refreshDisks().catch(() => undefined),
      pending ? 3000 : 30000,
    );
    return () => clearInterval(t);
  }, [busy, pending, refreshDisks]);

  const system = disks.find((d) => d.home) ?? disks[0];
  const others = disks.filter((d) => d !== system);

  /** Re-measure what is left after any action; no rescan. */
  const recheck = useCallback(async () => {
    try {
      const r = await api.recheck();
      setDisks(r.disks);
      const fresh = new Map(r.cards.map((c) => [c.id, c]));
      const gone = new Set(r.gone);
      const update = (list: Slot[] | null) =>
        list &&
        list.map((s) => {
          if (s.state !== "open") return s;
          if (gone.has(s.card.id)) return { ...s, state: "gone" as const };
          const c = fresh.get(s.card.id);
          return c ? { ...s, card: c } : s;
        });
      setCleanup(update);
      setPlacement(update);
      if (r.gone.length) {
        // Let the faded cards be seen, then fold them away.
        setTimeout(() => {
          const drop = (list: Slot[] | null) =>
            list && list.filter((s) => s.state !== "gone");
          setCleanup(drop);
          setPlacement(drop);
        }, 2600);
      }
    } catch {
      /* the cards stay as they were */
    }
  }, []);

  const celebrate = (
    bytes: number,
    verb: string,
    title: string,
    after: Disk[],
    undoId?: string | null,
    cardId?: string,
  ) => {
    const sys = after.find((d) => d.home) ?? after[0];
    resultKey.current += 1;
    setResult({
      key: resultKey.current,
      bytes,
      verb,
      title,
      free: sys?.free ?? 0,
      undo: undoId,
      card: cardId,
    });
    setGained((g) =>
      Math.max(0, verb === "Moved back" ? g - bytes : g + bytes),
    );
    setGlow(true);
    setTimeout(() => setGlow(false), 1400);
  };

  const open = async (slot: Slot) => {
    const card = slot.card;
    if (card.action === "command") {
      setConfirmRun(card);
      return;
    }
    setPlanning(card.id);
    try {
      const { plan, home: h } = await api.plan(card.id);
      setHome(h);
      setPreview({ plan, card });
    } catch (e) {
      setNote(errorText(e));
    } finally {
      setPlanning(null);
    }
  };

  const confirm = async (paths: string[]) => {
    if (!preview) return;
    const card = preview.card;
    try {
      const r = await api.run(card.id, paths);
      setDisks(r.disks);
      setUndo(r.undo);
      patch(card.id, { state: "done", outcome: r.outcome });
      setPreview(null);
      if (r.outcome.freed > 0)
        celebrate(
          r.outcome.freed,
          card.action === "move" ? "Moved" : "Freed",
          card.action === "move" ? " to the HDD" : "",
          r.disks,
          r.outcome.undo,
          card.id,
        );
      if (r.outcome.failed.length) setNote(r.outcome.failed[0]);
      recheck();
    } catch (e) {
      setPreview(null);
      setNote(errorText(e));
    }
  };

  const undoMove = async (undoId: string, cardId?: string) => {
    try {
      const r = await api.undo(undoId);
      setDisks(r.disks);
      setUndo(r.undo);
      if (cardId) patch(cardId, { state: "undone" });
      if (r.outcome.done > 0)
        celebrate(r.outcome.freed, "Moved back", " to the SSD", r.disks);
      else setResult(null);
      if (r.outcome.failed.length) setNote(r.outcome.failed[0]);
      recheck();
    } catch (e) {
      setNote(errorText(e));
    }
  };

  const startRun = async (card: Card) => {
    const before = system?.free ?? 0;
    const { cols, rows } = terminalSize();
    const r = await api.exec(card.id, cols, rows);
    setConfirmRun(null);
    setTerm({
      run: r.run,
      line: r.line,
      title: card.issue,
      code: null,
      done: false,
    });
    const finished = async (code: number) => {
      const after = await refreshDisks().catch(() => disks);
      const sys = after.find((d) => d.home) ?? after[0];
      const freed = Math.max(0, (sys?.free ?? before) - before);
      patch(card.id, {
        state: code === 0 ? "ran" : "open",
        note:
          code === 0
            ? freed > 1 << 20
              ? `Freed ${size(freed)}`
              : "Ran, exit 0"
            : undefined,
      });
      if (code === 0 && freed > 1 << 20) celebrate(freed, "Freed", "", after);
      if (code !== 0)
        setNote(`${card.issue}: exit ${code}. Output stays in the terminal.`);
      recheck();
    };
    runDone.current = finished;
  };
  const runDone = useRef<(code: number) => void>(() => undefined);

  const send = async () => {
    const text = draft.trim();
    if (!text || busy || cleanup === null) return;
    setDraft("");
    setChat({ q: text, a: null });
    try {
      const r = await api.chat(engine, text, history.slice(-8));
      setChat({ q: text, a: r.reply });
      setHistory((h) => [
        ...h,
        { role: "owner", text },
        { role: "assistant", text: r.reply },
      ]);
      if (r.suggestion) {
        const slot: Slot = { card: r.suggestion, state: "open" };
        if (r.suggestion.action === "move")
          setPlacement((p) => [slot, ...(p ?? [])]);
        else setCleanup((c) => [slot, ...(c ?? [])]);
      }
    } catch (e) {
      setChat({ q: text, a: errorText(e) });
    }
  };

  const ssdSegs = useMemo(
    () => (system ? segments(system.kinds, system.used, system.free) : []),
    [system],
  );
  const legendKeys = useMemo(
    () => present(disks.map((d) => segments(d.kinds, d.used, d.free))),
    [disks],
  );
  const topKinds = [...ssdSegs]
    .filter((s) => s.key !== "system" && s.key !== "other")
    .sort((a, b) => b.bytes - a.bytes)
    .slice(0, 3);
  const freeShown = useCountUp(system?.free ?? 0, 1100);
  const full = system
    ? Math.round(usedRatio(system.used + system.free, system.free) * 100)
    : 0;
  const hddFree = others.reduce((s, d) => s + d.free, 0);
  const openCleanup = (cleanup ?? []).filter((s) => s.state === "open");
  const openPlace = (placement ?? []).filter((s) => s.state === "open");
  const cleanupLeft = openCleanup.reduce((s, x) => s + x.card.bytes, 0);
  const tidy =
    cleanup !== null &&
    placement !== null &&
    openCleanup.length === 0 &&
    openPlace.length === 0;
  const firstOpen = (list: Slot[] | null) =>
    list?.find((s) => s.state === "open")?.card.id;

  return (
    <div className={term ? "app with-term" : "app"}>
      <header className="top">
        <span className="brand">
          <Logo />
          Mem Inspector
        </span>
        <span className="top-right">
          <span className="scanned tabular">
            {busy
              ? "Scanning"
              : scannedAt
                ? `Scanned ${ago(scannedAt, now)}`
                : ""}
          </span>
          <button
            className="btn solid"
            onClick={() => start(true)}
            disabled={!!busy}
          >
            <RotateCw
              size={14}
              strokeWidth={1.75}
              className={busy ? "spin" : ""}
            />
            Rescan
          </button>
          <span className="gear-wrap">
            <button
              className={gear ? "icon-btn on" : "icon-btn"}
              aria-label="Settings"
              aria-expanded={gear}
              onClick={() => setGear((g) => !g)}
            >
              <Settings2 size={16} strokeWidth={1.75} />
            </button>
            {gear && (
              <Settings
                engine={engine}
                setEngine={setEngine}
                undo={undo}
                onUndo={(u) => {
                  setGear(false);
                  undoMove(u);
                }}
                onClose={() => setGear(false)}
              />
            )}
          </span>
        </span>
      </header>

      <div className="lanes">
        <section className="lane ssd">
          <div className="lane-title">
            <Zap size={14} strokeWidth={1.75} />
            {system ? system.name.toUpperCase() : "SYSTEM"} SSD
            <span className="speed">
              fast{system ? ` · ${size(system.total)}` : ""}
            </span>
          </div>
          {system ? (
            <>
              <div className="hero">
                <div>
                  <b className={glow ? "free ok" : "free"}>
                    {size(freeShown)} free
                  </b>
                  <small className="tabular">
                    {gained > 0
                      ? `+${size(gained)} this session`
                      : `${full}% full`}
                  </small>
                </div>
                <div className="what tabular">
                  {topKinds
                    .map((k) => `${k.label} ${size(k.bytes)}`)
                    .join(" · ")}
                </div>
              </div>
              <div className={glow ? "bar-wrap glow" : "bar-wrap"}>
                <Bar disk={system} big focus={focus} />
              </div>
              {legendKeys.length > 0 && (
                <Legend keys={legendKeys} focus={focus} setFocus={setFocus} />
              )}
            </>
          ) : (
            <div className="hero placeholder" />
          )}
          {result && (
            <ResultBanner
              result={result}
              onUndo={() => result.undo && undoMove(result.undo, result.card)}
            />
          )}
          {scan ? (
            <Scanning
              folders={scan.folders}
              measured={scan.measured}
              since={busy?.since ?? now}
              now={now}
            />
          ) : tidy ? (
            <div className="empty">
              <span className="freed-mark">
                <svg viewBox="0 0 24 24">
                  <path d="M20 6 9 17l-5-5" />
                </svg>
              </span>
              <div>
                <b>All tidy</b>
                <p className="quiet">Nothing big to clean or move right now.</p>
              </div>
            </div>
          ) : (
            <>
              <div className="sub">
                FREE UP SPACE
                <span className="tabular">
                  {cleanup === null
                    ? ""
                    : openCleanup.length
                      ? `${size(cleanupLeft)} possible`
                      : "Nothing to clean"}
                </span>
              </div>
              {cleanup === null ? (
                <Skeleton n={3} label="Claude is ranking the biggest wins" />
              ) : (
                cleanup.map((s, i) => (
                  <CardView
                    key={s.card.id}
                    slot={s}
                    home={home}
                    archive={archive}
                    index={i}
                    primary={s.card.id === firstOpen(cleanup)}
                    loading={planning === s.card.id}
                    onDo={() => open(s)}
                    onSkip={() => patch(s.card.id, { state: "skipped" })}
                    onUndo={() =>
                      s.outcome?.undo && undoMove(s.outcome.undo, s.card.id)
                    }
                  />
                ))
              )}
              {note && <p className="hint">{note}</p>}
            </>
          )}
        </section>

        <section className="lane hdd">
          <div className="lane-title">
            <HardDrive size={14} strokeWidth={1.75} />
            HDD
            <span className="speed">
              slow, roomy{others.length ? ` · ${size(hddFree)} free` : ""}
            </span>
          </div>
          {others.map((d) => (
            <div key={d.mount} className="hdd-disk">
              <span className="n">
                {d.name}
                <span>{d.mount}</span>
              </span>
              <span className="f tabular">
                <CountSize bytes={d.free} /> free
              </span>
              <Bar disk={d} focus={focus} />
            </div>
          ))}
          {!scan && (
            <>
              <div className="sub">
                BELONGS ON THE HDD
                <span>Moved with a link back, undoable</span>
              </div>
              {placement === null ? (
                <Skeleton n={2} label="Looking for data that belongs here" />
              ) : placement.length === 0 ? (
                <p className="hint">Everything is on the right disk.</p>
              ) : (
                placement.map((s, i) => (
                  <CardView
                    key={s.card.id}
                    slot={s}
                    home={home}
                    archive={archive}
                    index={i}
                    primary={s.card.id === firstOpen(placement)}
                    loading={planning === s.card.id}
                    onDo={() => open(s)}
                    onSkip={() => patch(s.card.id, { state: "skipped" })}
                    onUndo={() =>
                      s.outcome?.undo && undoMove(s.outcome.undo, s.card.id)
                    }
                  />
                ))
              )}
              {keep && (
                <p className="keep">
                  <Zap size={13} strokeWidth={1.75} />
                  {keep}
                </p>
              )}
            </>
          )}
        </section>
      </div>

      {term && (
        <TerminalPanel
          state={term}
          onUpdate={setTerm}
          onFinished={(code) => runDone.current(code)}
          onClose={() => setTerm(null)}
        />
      )}

      <footer className="dock">
        <div className="last">
          {chat && (
            <>
              <span className="owner-pill">{chat.q}</span>
              <p>{chat.a ?? <Spinner />}</p>
            </>
          )}
        </div>
        <form
          className="composer"
          onSubmit={(e) => {
            e.preventDefault();
            send();
          }}
        >
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={
              cleanup === null
                ? "Ask once the scan is done"
                : "Ask about your disks"
            }
            disabled={cleanup === null}
            aria-label="Message"
          />
          <button
            type="submit"
            className={draft.trim() ? "send ready" : "send"}
            disabled={!draft.trim() || cleanup === null}
            aria-label="Send"
          >
            <ArrowUp size={16} strokeWidth={2} />
          </button>
        </form>
      </footer>

      {preview && (
        <Preview
          plan={preview.plan}
          card={preview.card}
          home={home}
          onClose={() => setPreview(null)}
          onConfirm={confirm}
        />
      )}
      {confirmRun && (
        <ConfirmRun
          card={confirmRun}
          onClose={() => setConfirmRun(null)}
          onRun={async () => {
            try {
              await startRun(confirmRun);
            } catch (e) {
              setConfirmRun(null);
              setNote(errorText(e));
            }
          }}
        />
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
