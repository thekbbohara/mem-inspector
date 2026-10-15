import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { CircleStop, SquareTerminal, X } from "lucide-react";
import { api } from "./api";
import { Spinner } from "./ui";

export type RunState = {
  run: string;
  line: string;
  title: string;
  code: number | null;
  done: boolean;
};

const theme = {
  background: "#0b0b0d",
  foreground: "#eae7e2",
  cursor: "#0b0b0d",
  selectionBackground: "rgba(232,121,58,0.28)",
  black: "#18181c",
  brightBlack: "#616167",
  red: "#e66767",
  green: "#7fb98f",
  yellow: "#c98500",
  blue: "#3987e5",
  magenta: "#9085e9",
  cyan: "#199e70",
  white: "#eae7e2",
};

/** Size the PTY to the panel before the command starts. */
export function terminalSize(): { cols: number; rows: number } {
  const width = Math.max(320, window.innerWidth - 96);
  return { cols: Math.floor(width / 7.3), rows: 12 };
}

function decode(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/**
 * Live output of one app command in a real terminal emulator. Output stays
 * for scrolling after the command exits; closing the panel is the owner's call.
 */
export function TerminalPanel({
  state,
  onUpdate,
  onFinished,
  onClose,
}: {
  state: RunState;
  onUpdate: (s: RunState) => void;
  onFinished: (code: number) => void;
  onClose: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [stopping, setStopping] = useState(false);
  useEffect(() => {
    const term = new Terminal({
      theme,
      fontFamily: 'ui-monospace, "JetBrains Mono", "SF Mono", Menlo, monospace',
      fontSize: 12,
      lineHeight: 1.25,
      convertEol: false,
      disableStdin: true,
      cursorBlink: false,
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host.current!);
    fit.fit();
    const onResize = () => fit.fit();
    window.addEventListener("resize", onResize);
    let offset = 0;
    let alive = true;
    let timer = 0;
    const poll = async () => {
      try {
        const chunk = await api.term(state.run, offset);
        if (!alive) return;
        if (chunk.data) term.write(decode(chunk.data));
        offset = chunk.offset;
        if (chunk.done) {
          const code = chunk.code ?? -1;
          term.write(
            code === 0
              ? `\r\n\x1b[32m✓ exit 0\x1b[0m\r\n`
              : `\r\n\x1b[31mexit ${code}\x1b[0m\r\n`,
          );
          onUpdate({ ...state, done: true, code });
          onFinished(code);
          return;
        }
      } catch (e) {
        term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`);
        return;
      }
      timer = window.setTimeout(poll, 120);
    };
    poll();
    return () => {
      alive = false;
      clearTimeout(timer);
      window.removeEventListener("resize", onResize);
      term.dispose();
    };
    // One terminal per run.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.run]);
  const ok = state.done && state.code === 0;
  return (
    <section className="term-panel" aria-label="Command output">
      <header>
        <SquareTerminal size={15} strokeWidth={1.75} />
        <span className="term-title">{state.title}</span>
        <code className="term-line">{state.line}</code>
        <span
          className={`term-status${state.done ? (ok ? " ok" : " bad") : ""}`}
        >
          {state.done ? (
            ok ? (
              "Done · exit 0"
            ) : (
              `Exit ${state.code}`
            )
          ) : (
            <>
              <Spinner />
              Running
            </>
          )}
        </span>
        {state.done ? (
          <button
            className="icon-btn"
            onClick={onClose}
            aria-label="Close terminal"
          >
            <X size={15} strokeWidth={1.75} />
          </button>
        ) : (
          <button
            className="btn ghost"
            disabled={stopping}
            onClick={async () => {
              setStopping(true);
              await api.stop(state.run).catch(() => undefined);
            }}
          >
            <CircleStop size={14} strokeWidth={1.75} />
            Stop
          </button>
        )}
      </header>
      <div className="term-host" ref={host} />
    </section>
  );
}
