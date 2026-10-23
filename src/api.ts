import { requestNative } from "./bridge";

export type Action = "delete" | "move" | "command";
export type Disk = {
  name: string;
  mount: string;
  medium: string;
  total: number;
  free: number;
  used: number;
  home: boolean;
  kinds: Record<string, number> | null;
  pending: boolean;
};
export type Card = {
  id: string;
  issue: string;
  fix: string;
  action: Action;
  paths: string[];
  bytes: number;
  command?: string | null;
  kind: string;
  root?: boolean;
  group?: string;
  target?: string;
  tradeoff?: string;
  /** Shown with its reason in `fix`, never run (a VM disk in use). */
  held?: boolean;
};
export type Row = { path: string; bytes: number; ok?: boolean };
export type Plan = {
  id: string;
  action: Action;
  rows: Row[];
  target: string | null;
  command?: string | null;
};
export type Outcome = {
  freed: number;
  done: number;
  failed: string[];
  undo: string | null;
};
export type Undo = { id: string; count: number; bytes: number };
export type Engine = { provider: string; model: string };
export type Turn = { role: "owner" | "assistant"; text: string };
export type Folder = { path: string; allocated: string };
export type Overview = {
  overview?: {
    complete?: boolean;
    seconds?: number;
    allocated?: string;
    folders?: Folder[];
  } | null;
  pending?: boolean;
};
export type TermChunk = {
  data: string;
  offset: number;
  done: boolean;
  code: number | null;
  card: string;
  line: string;
};
type DiskState = { disks: Disk[]; undo: Undo[] };
export type AppSettings = {
  /** Folder on a second disk for personal files; null means moves are off. */
  archive: string | null;
  setupDone: boolean;
  home: string;
  claude: { installed: boolean; dir: string; configured: boolean };
};

const call = <T>(request: Record<string, unknown>) =>
  requestNative(request as never) as Promise<T>;

const engine = (e: Engine) => ({
  provider: e.provider,
  settings: { model: e.model, account: "" },
});

export const api = {
  settings: () => call<AppSettings>({ action: "settings" }),
  /** Native folder picker; null when cancelled. */
  pick: () => call<{ path: string | null }>({ action: "pick" }),
  /** Checks and saves the folder; null turns moves off. */
  setArchive: (path: string | null) =>
    call<AppSettings>({ action: "setarchive", path }),
  overview: (fresh = false) =>
    call<Overview>({
      action: "overview",
      ...(fresh ? { confirmed: true } : {}),
    }),
  disks: () =>
    call<
      DiskState & {
        home: string;
        archive: string | null;
        /** Set when the archive drive is unplugged: moves are paused. */
        paused: string | null;
        scannedAt: number | null;
      }
    >({ action: "disks" }),
  suggest: (e: Engine) =>
    call<{
      suggestions: Card[];
      source: string;
      note: string | null;
      home: string;
    }>({ action: "suggest", ...engine(e) }),
  placement: () =>
    call<{
      cards: Card[];
      keep: string;
      archive: string | null;
      paused: string | null;
      home: string;
    }>({
      action: "placement",
    }),
  recheck: () =>
    call<{ cards: Card[]; gone: string[]; disks: Disk[] }>({
      action: "recheck",
    }),
  progress: () => call<{ done: number; total: number }>({ action: "progress" }),
  chat: (e: Engine, text: string, history: Turn[]) =>
    call<{ reply: string; suggestion: Card | null; source: string }>({
      action: "chat",
      ...engine(e),
      text,
      history,
    }),
  plan: (id: string) =>
    call<{ plan: Plan; home: string }>({ action: "plan", id }),
  run: (id: string, paths: string[]) =>
    call<DiskState & { outcome: Outcome }>({ action: "run", id, paths }),
  undo: (id: string) =>
    call<DiskState & { outcome: Outcome }>({ action: "undo", id }),
  /** Starts the card's own command; the backend refuses without confirm. */
  exec: (id: string, cols: number, rows: number) =>
    call<{ run: string; line: string; card: string }>({
      action: "exec",
      id,
      confirmed: true,
      cols,
      rows,
    }),
  term: (id: string, offset: number) =>
    call<TermChunk>({ action: "term", id, offset }),
  stop: (id: string) => call<TermChunk>({ action: "stop", id, offset: 0 }),
};
