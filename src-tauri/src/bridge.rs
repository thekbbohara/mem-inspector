use crate::advisor::{self, Facts, Suggestion};
use crate::placement;
use crate::space::{self, Kind as Action2, Space};
use crate::tools;
use mem_core::{Cancellation, Limits, Queue, QueueOptions, Scan};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Status,
    Overview,
    Choose,
    Scan,
    Cancel,
    Providers,
    Preview,
    Discard,
    Commit,
    History,
    Restore,
    Duplicates,
    Advice,
    Propose,
    Disks,
    Suggest,
    Chat,
    Plan,
    Run,
    Undo,
    Placement,
    Recheck,
    Progress,
    Exec,
    Term,
    Stop,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub action: Action,
    pub path: Option<String>,
    pub paths: Option<Vec<String>>,
    pub id: Option<String>,
    pub confirmed: Option<bool>,
    pub provider: Option<String>,
    pub settings: Option<mem_ai::EngineSettings>,
    pub consent: Option<bool>,
    pub verified: Option<Vec<String>>,
    pub excluded: Option<Vec<String>>,
    pub text: Option<String>,
    pub history: Option<Vec<crate::advisor::Turn>>,
    pub offset: Option<usize>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}
#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}
struct Inner {
    data: Mutex<Data>,
    active: Mutex<Option<Cancellation>>,
    fixture: Option<(PathBuf, PathBuf)>,
    overview: Mutex<Option<Value>>,
    overview_running: Mutex<bool>,
    space: Option<Space>,
    facts: Mutex<Option<Facts>>,
    gathering: Mutex<()>,
    cards: Mutex<Vec<Suggestion>>,
    /// Used space by data type on the other disks, keyed by mount.
    disk_kinds: Mutex<HashMap<String, Value>>,
    disk_kinds_running: Mutex<bool>,
    /// Bytes moved or freed since each disk was last measured, by mount and type.
    deltas: Mutex<HashMap<String, HashMap<String, i64>>>,
    /// Data type of each archive batch, so undo can move it back on the bars.
    undo_kinds: Mutex<HashMap<String, String>>,
    runs: Mutex<Vec<Arc<tools::Run>>>,
}

static CARD_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
#[derive(Default)]
struct Data {
    last: Option<Scan>,
    queues: HashMap<String, Queue>,
}
struct Job<'a>(&'a Mutex<Option<Cancellation>>);
impl Drop for Job<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}
fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Internal state unavailable".into())
}
// u64 identifiers and byte counts cross JavaScript as decimal strings without precision loss.
fn wire<T: serde::Serialize>(input: T) -> Result<Value, String> {
    let mut v = serde_json::to_value(input).map_err(|e| e.to_string())?;
    stringify(&mut v);
    Ok(v)
}
fn stringify(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if matches!(
                    k.as_str(),
                    "apparent" | "allocated" | "dev" | "ino" | "total" | "free"
                ) {
                    if let Some(n) = v.as_u64() {
                        *v = Value::String(n.to_string());
                    }
                } else {
                    stringify(v)
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                stringify(v)
            }
        }
        _ => {}
    }
}
impl Bridge {
    pub fn from_env() -> Result<Self, String> {
        let fixture = match (
            std::env::var_os("MEM_INSPECTOR_FIXTURE_ROOT"),
            std::env::var_os("MEM_INSPECTOR_FIXTURE_TRASH"),
        ) {
            (None, None) => None,
            (Some(root), Some(trash)) => {
                let root = PathBuf::from(root);
                let trash = PathBuf::from(trash);
                if !root.is_absolute()
                    || !trash.is_absolute()
                    || root.parent() != trash.parent()
                    || !root
                        .file_name()
                        .and_then(|p| p.to_str())
                        .is_some_and(|p| p.starts_with("mem-core-fixture-"))
                    || !root.join(".mem-inspector-fixture").is_file()
                {
                    return Err(
                        "Fixture root and sibling Trash need an explicit disposable fixture marker"
                            .into(),
                    );
                }
                Some((root, trash))
            }
            _ => return Err("Both fixture root and fixture Trash are required".into()),
        };
        Ok(Self::new(fixture))
    }
    fn new(fixture: Option<(PathBuf, PathBuf)>) -> Self {
        Self::with_space(fixture, Space::from_env().ok())
    }
    fn with_space(fixture: Option<(PathBuf, PathBuf)>, space: Option<Space>) -> Self {
        Self {
            inner: Arc::new(Inner {
                data: Mutex::new(Data::default()),
                active: Mutex::new(None),
                fixture,
                overview: Mutex::new(None),
                overview_running: Mutex::new(false),
                space,
                facts: Mutex::new(None),
                gathering: Mutex::new(()),
                cards: Mutex::new(Vec::new()),
                disk_kinds: Mutex::new(load_disk_kinds()),
                disk_kinds_running: Mutex::new(false),
                deltas: Mutex::new(HashMap::new()),
                undo_kinds: Mutex::new(HashMap::new()),
                runs: Mutex::new(Vec::new()),
            }),
        }
    }
    pub fn fixture_root(&self) -> Option<String> {
        self.inner
            .fixture
            .as_ref()
            .map(|(root, _)| root.to_string_lossy().into_owned())
    }
    fn history(data: &Data) -> Result<Value, String> {
        wire(
            data.queues
                .values()
                .flat_map(|q| q.history().iter())
                .collect::<Vec<_>>(),
        )
    }
    pub fn request(&self, r: Request) -> Result<Value, String> {
        if matches!(r.action, Action::Overview) {
            let root = self
                .fixture_root()
                .or_else(|| {
                    self.inner
                        .space
                        .as_ref()
                        .map(|s| s.home.to_string_lossy().into_owned())
                })
                .ok_or("Home directory unavailable")?;
            let mut running = lock(&self.inner.overview_running)?;
            if !*running && (lock(&self.inner.overview)?.is_none() || r.confirmed == Some(true)) {
                *running = true;
                *lock(&self.inner.overview)? = None;
                let inner = self.inner.clone();
                std::thread::spawn(move || {
                    use std::io::{BufRead, BufReader};
                    let job = (|| -> Result<(), String> {
                        let mut child = std::process::Command::new("python3")
                            .arg("-c")
                            .arg(include_str!("../../scripts/storage-overview.py"))
                            .arg(root)
                            .env("MEM_OVERVIEW_STREAM", "1")
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped())
                            .spawn()
                            .map_err(|e| format!("Overview requires Python 3 and GNU du: {e}"))?;
                        for line in
                            BufReader::new(child.stdout.take().ok_or("Missing scan output")?)
                                .lines()
                        {
                            let value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                                .map_err(|e| e.to_string())?;
                            *lock(&inner.overview)? = Some(value);
                        }
                        let output = child.wait_with_output().map_err(|e| e.to_string())?;
                        if !output.status.success() {
                            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
                        }
                        Ok(())
                    })();
                    let ok = job.is_ok();
                    if let Err(error) = job {
                        if let Ok(mut state) = inner.overview.lock() {
                            *state = Some(json!({"error": error}));
                        }
                    }
                    if let Ok(mut running) = inner.overview_running.lock() {
                        *running = false;
                    }
                    if ok && inner.fixture.is_none() {
                        if let Ok(mut d) = inner.deltas.lock() {
                            d.clear();
                        }
                        scan_other_disks(&inner);
                    }
                });
            }
            let state = lock(&self.inner.overview)?.clone();
            if let Some(error) = state.as_ref().and_then(|v| v.get("error")) {
                return Err(error.as_str().unwrap_or("Scan failed").to_string());
            }
            return Ok(json!({"overview": state, "pending": *running}));
        }
        if matches!(
            r.action,
            Action::Disks
                | Action::Suggest
                | Action::Chat
                | Action::Plan
                | Action::Run
                | Action::Undo
                | Action::Placement
                | Action::Recheck
                | Action::Progress
                | Action::Exec
                | Action::Term
                | Action::Stop
        ) {
            return self.simple(r);
        }
        if matches!(r.action, Action::Status) {
            return Ok(json!({"root":self.fixture_root(),"fixture":self.inner.fixture.is_some()}));
        }
        if matches!(r.action, Action::Cancel) {
            if let Some(c) = lock(&self.inner.active)?.as_ref() {
                c.cancel();
            }
            return Ok(json!({"message":"Cancellation requested"}));
        }
        if matches!(r.action, Action::History) {
            return Ok(json!({"history":Self::history(&*lock(&self.inner.data)?)?}));
        }
        if matches!(r.action, Action::Advice | Action::Propose) && r.consent != Some(true) {
            return Err("Explicit consent required for each advice request".into());
        }
        if matches!(r.action, Action::Providers) {
            return Ok(
                json!({"providers":mem_ai::discover_with(r.provider.as_deref(), r.settings.as_ref()),"fixture":self.inner.fixture.is_some()}),
            );
        }
        if matches!(r.action, Action::Discard) {
            let mut data = lock(&self.inner.data)?;
            let id = r.id.as_deref().ok_or("Review ID required")?;
            for queue in data.queues.values_mut() {
                queue.discard_preview(id);
            }
            return Ok(json!({"history":Self::history(&data)?}));
        }
        let cancel = Cancellation::default();
        {
            let mut active = lock(&self.inner.active)?;
            if active.is_some() {
                return Err("Another operation is running".into());
            }
            *active = Some(cancel.clone());
        }
        let _job = Job(&self.inner.active);
        match r.action {
            Action::Providers => Ok(
                json!({"providers":mem_ai::discover_with(r.provider.as_deref(), r.settings.as_ref()),"fixture":self.inner.fixture.is_some()}),
            ),
            Action::Overview
            | Action::Disks
            | Action::Suggest
            | Action::Chat
            | Action::Plan
            | Action::Run
            | Action::Undo
            | Action::Placement
            | Action::Recheck
            | Action::Progress
            | Action::Exec
            | Action::Term
            | Action::Stop => unreachable!(),
            Action::Scan => {
                let root = match &self.inner.fixture {
                    Some((root, _)) => {
                        if r.path.as_deref().is_some_and(|p| Path::new(p) != root) {
                            return Err(
                                "Fixture mode restricts scans to the disposable fixture root"
                                    .into(),
                            );
                        }
                        root.clone()
                    }
                    None => r.path.map(PathBuf::from).unwrap_or_else(|| {
                        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                            .join("Downloads")
                    }),
                };
                let scan = mem_core::scan(&root, &Limits::default(), &cancel)
                    .map_err(|e| e.to_string())?;
                let storage = storage(&root);
                let evidence = local_evidence(&scan, self.inner.fixture.as_ref());
                let result = json!({"scan":wire(&scan)?,"evidence":evidence,"storage":storage,"fixture":self.inner.fixture.is_some()});
                lock(&self.inner.data)?.last = Some(scan);
                Ok(result)
            }
            Action::Duplicates => {
                let data = lock(&self.inner.data)?;
                let scan = data.last.as_ref().ok_or("Scan a folder first")?;
                let groups = mem_core::duplicates(&scan.entries, &Limits::default(), &cancel)
                    .map_err(|e| e.to_string())?;
                Ok(json!({"duplicates":groups}))
            }
            Action::Propose => {
                let data = lock(&self.inner.data)?;
                let scan = data.last.as_ref().ok_or("Scan a folder first")?;
                if scan.cancelled || scan.truncated || !scan.errors.is_empty() {
                    return Err("Complete an error-free scan before preparing cleanup".into());
                }
                let entries: Vec<_> = scan
                    .entries
                    .iter()
                    .filter(|e| e.kind == "file" && !e.shared)
                    .take(500)
                    .collect();
                let verified = r.verified.as_deref().unwrap_or(&[]);
                let excluded = r.excluded.as_deref().unwrap_or(&[]);
                if verified
                    .iter()
                    .any(|p| !entries.iter().any(|e| &e.path == p))
                {
                    return Err("Unknown confirmed item".into());
                }
                let evidence = local_evidence(scan, self.inner.fixture.as_ref());
                let items: Vec<_> = entries
                    .iter()
                    .enumerate()
                    .map(|(id, e)| mem_ai::ProposalItem {
                        id: id as u32,
                        bytes: e.allocated,
                        evidence: if excluded.contains(&e.path) {
                            mem_ai::Evidence::Unknown
                        } else if evidence.get(&e.path) == Some(&mem_ai::Evidence::CacheIntent) {
                            mem_ai::Evidence::CacheIntent
                        } else if verified.contains(&e.path)
                            && backend_eligible(scan, &e.path, self.inner.fixture.as_ref())
                        {
                            mem_ai::Evidence::UserConfirmed
                        } else {
                            mem_ai::Evidence::Unknown
                        },
                    })
                    .collect();
                let proposal = mem_ai::propose_with(
                    r.provider.as_deref().unwrap_or(""),
                    &items,
                    true,
                    r.settings.as_ref(),
                )
                .map_err(|e| e.to_string())?;
                let paths: Vec<_> = proposal
                    .selected
                    .iter()
                    .map(|id| entries[*id as usize].path.clone())
                    .collect();
                if paths.iter().any(|p| {
                    !backend_eligible(scan, p, self.inner.fixture.as_ref())
                        || (!verified.contains(p)
                            && !crate::evidence::cache_intent(Path::new(&scan.root), Path::new(p)))
                }) {
                    return Err("Proposal evidence or file identity changed; scan again".into());
                }
                Ok(
                    json!({"proposal":{"provider":r.provider,"paths":paths,"considered":items.len(),"evidence":evidence}}),
                )
            }
            Action::Advice => {
                let summary = {
                    let data = lock(&self.inner.data)?;
                    aggregate(data.last.as_ref().ok_or("Scan a folder first")?)
                };
                let advice = mem_ai::advice_with(
                    r.provider.as_deref().unwrap_or(""),
                    &summary,
                    true,
                    r.settings.as_ref(),
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({"advice":advice}))
            }
            Action::Preview => {
                let mut data = lock(&self.inner.data)?;
                let root = data
                    .last
                    .as_ref()
                    .ok_or("Scan a folder first")?
                    .root
                    .clone();
                if !data.queues.contains_key(&root) {
                    let trash = match &self.inner.fixture {
                        Some((_, trash)) => trash.clone(),
                        None => home_trash()?,
                    };
                    let q = Queue::new(QueueOptions {
                        root: PathBuf::from(&root),
                        trash,
                        protected_paths: vec![],
                        fixture: self.inner.fixture.is_some(),
                    })
                    .map_err(|e| e.to_string())?;
                    data.queues.insert(root.clone(), q);
                }
                let preview = data
                    .queues
                    .get_mut(&root)
                    .ok_or("Queue unavailable")?
                    .preview(r.paths.as_deref().unwrap_or(&[]))
                    .map_err(|e| e.to_string())?;
                Ok(json!({"preview":wire(preview)?}))
            }
            Action::Commit | Action::Discard | Action::Restore => {
                let id = r.id.as_deref().ok_or("Review or history ID required")?;
                if matches!(r.action, Action::Commit | Action::Restore) && r.confirmed != Some(true)
                {
                    return Err("Explicit confirmation required".into());
                }
                let mut data = lock(&self.inner.data)?;
                let root = if matches!(r.action, Action::Restore) {
                    data.queues
                        .iter()
                        .find(|(_, q)| q.history().iter().any(|h| h.id == id))
                        .map(|(root, _)| root.clone())
                        .ok_or("Unknown history")?
                } else {
                    data.last
                        .as_ref()
                        .ok_or("Scan a folder first")?
                        .root
                        .clone()
                };
                let queue = data.queues.get_mut(&root).ok_or("Queue unavailable")?;
                match r.action {
                    Action::Commit => {
                        queue.commit(id, true, &cancel).map_err(|e| e.to_string())?;
                    }
                    Action::Restore => {
                        queue.restore(id, &cancel).map_err(|e| e.to_string())?;
                    }
                    Action::Discard => {
                        queue.discard_preview(id);
                    }
                    _ => unreachable!(),
                };
                Ok(json!({"history":Self::history(&data)?}))
            }
            Action::Status | Action::Choose | Action::Cancel | Action::History => {
                Err("Unsupported dispatch".into())
            }
        }
    }
}
impl Bridge {
    fn space(&self) -> Result<&Space, String> {
        self.inner
            .space
            .as_ref()
            .ok_or_else(|| "Home or archive configuration invalid".into())
    }
    fn complete_overview(&self) -> Result<Value, String> {
        let overview = lock(&self.inner.overview)?.clone();
        match overview {
            Some(v) if v["complete"] == true && !*lock(&self.inner.overview_running)? => Ok(v),
            _ => Err("Scan still running".into()),
        }
    }
    /// Facts for the latest finished scan, gathered once per scan even when
    /// suggestions and placement ask at the same time.
    fn facts(&self) -> Result<Facts, String> {
        let overview = self.complete_overview()?;
        let scanned = overview["scannedAt"].as_i64().unwrap_or(0);
        let _one = lock(&self.inner.gathering)?;
        if let Some(f) = lock(&self.inner.facts)?.clone() {
            if f.scanned_at == scanned {
                return Ok(f);
            }
        }
        let f = advisor::gather(self.space()?, &overview);
        *lock(&self.inner.facts)? = Some(f.clone());
        Ok(f)
    }
    fn register(&self, prefix: &str, mut card: Suggestion) -> Result<Suggestion, String> {
        let n = CARD_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        card.id = format!("{prefix}{n}");
        lock(&self.inner.cards)?.push(card.clone());
        Ok(card)
    }
    fn mount_of(&self, path: &str, disks: &[advisor::Disk]) -> Option<String> {
        disks
            .iter()
            .filter(|d| Path::new(path).starts_with(&d.mount))
            .max_by_key(|d| d.mount.len())
            .map(|d| d.mount.clone())
    }
    /// Disks with live free space and used space split by data type.
    fn disk_view(&self) -> Result<Value, String> {
        let overview = lock(&self.inner.overview)?.clone().unwrap_or(Value::Null);
        let disks = advisor::disks(&overview);
        let home = self.space()?.home.to_string_lossy().into_owned();
        let home_mount = self.mount_of(&home, &disks);
        let complete = overview["complete"] == true && overview["kinds"].is_object();
        let others = lock(&self.inner.disk_kinds)?.clone();
        let deltas = lock(&self.inner.deltas)?.clone();
        let running = *lock(&self.inner.disk_kinds_running)?;
        let out: Vec<Value> = disks
            .iter()
            .map(|d| {
                let is_home = home_mount.as_deref() == Some(d.mount.as_str());
                let source = if is_home {
                    complete.then(|| overview["kinds"].clone())
                } else {
                    others.get(&d.mount).map(|v| v["kinds"].clone())
                };
                let kinds = source.map(|src| {
                    let mut kinds: serde_json::Map<String, Value> = serde_json::Map::new();
                    let mut sum = 0u64;
                    for k in KINDS {
                        let base = src[k].as_str().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                        let delta = deltas.get(&d.mount).and_then(|m| m.get(k)).copied().unwrap_or(0);
                        let v = (base + delta).max(0) as u64;
                        sum += v;
                        kinds.insert(k.into(), json!(v));
                    }
                    let rest = d.used.saturating_sub(sum);
                    if is_home {
                        kinds.insert("system".into(), json!(rest));
                    } else {
                        let other = kinds["other"].as_u64().unwrap_or(0) + rest;
                        kinds.insert("other".into(), json!(other));
                    }
                    Value::Object(kinds)
                });
                json!({
                    "name": d.name, "mount": d.mount, "medium": d.medium,
                    "total": d.total, "free": d.free, "used": d.used,
                    "home": is_home, "kinds": kinds,
                    "pending": kinds_pending(is_home, complete, running, others.contains_key(&d.mount)),
                })
            })
            .collect();
        Ok(json!(out))
    }
    /// Move bytes between data-type totals after an action, until the next scan.
    fn shift(
        &self,
        kind: &str,
        bytes: u64,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<(), String> {
        let overview = lock(&self.inner.overview)?.clone().unwrap_or(Value::Null);
        let disks = advisor::disks(&overview);
        let mut deltas = lock(&self.inner.deltas)?;
        for (path, sign) in [(from, -1i64), (to, 1)] {
            if let Some(m) = path.and_then(|p| self.mount_of(p, &disks)) {
                *deltas.entry(m).or_default().entry(kind.into()).or_default() +=
                    sign * bytes as i64;
            }
        }
        Ok(())
    }
    fn scanned_at(&self) -> Result<Value, String> {
        let overview = lock(&self.inner.overview)?;
        Ok(overview
            .as_ref()
            .filter(|o| o["complete"] == true)
            .map(|o| o["scannedAt"].clone())
            .unwrap_or(Value::Null))
    }
    fn undo_list(&self) -> Result<Value, String> {
        let space = self.space()?;
        Ok(json!(space
            .undoable()
            .into_iter()
            .map(|(id, items)| json!({"id": id, "count": items.len(), "bytes": items.iter().map(|m| m.bytes).sum::<u64>()}))
            .collect::<Vec<_>>()))
    }
    /// The chat-first surface: disks, cards, preview, run and undo.
    fn simple(&self, r: Request) -> Result<Value, String> {
        let space = self.space()?;
        let home = space.home.to_string_lossy().into_owned();
        match r.action {
            Action::Disks => Ok(
                json!({"disks": self.disk_view()?, "home": home, "archive": space.archive, "undo": self.undo_list()?, "scannedAt": self.scanned_at()?}),
            ),
            Action::Progress => {
                let (done, total) = space::progress();
                Ok(json!({"done": done, "total": total}))
            }
            Action::Placement => {
                let facts = self.facts()?;
                lock(&self.inner.cards)?.retain(|c| !c.id.starts_with('p'));
                let cards = advisor::placement_cards(&facts)
                    .into_iter()
                    .map(|c| self.register("p", c))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(
                    json!({"cards": cards, "keep": placement::KEEP_ON_SSD, "archive": space.archive, "home": home}),
                )
            }
            Action::Recheck => self.recheck(),
            Action::Exec => {
                let card = self.card(r.id.as_deref())?;
                let tool = (card.action == Action2::Command)
                    .then(|| card.paths.first().and_then(|p| tools::Tool::from_key(p)))
                    .flatten()
                    .ok_or("Only the app's own cleanup commands can run")?;
                let cmd = tool
                    .command(&space.home)
                    .ok_or("This command needs root; run it in your own terminal")?;
                if r.confirmed != Some(true) {
                    return Err("Confirm the exact command first".into());
                }
                let mut runs = lock(&self.inner.runs)?;
                if runs.iter().any(|run| run.done().is_none()) {
                    return Err("A command is already running".into());
                }
                let id = format!(
                    "r{}",
                    CARD_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                let line = tool.line(&space.home);
                let run = tools::Run::start(
                    id.clone(),
                    card.id.clone(),
                    line.clone(),
                    cmd,
                    &space.home,
                    r.cols.unwrap_or(100).clamp(20, 400),
                    r.rows.unwrap_or(16).clamp(4, 200),
                )?;
                runs.push(Arc::new(run));
                Ok(json!({"run": id, "line": line, "card": card.id}))
            }
            Action::Term | Action::Stop => {
                let id = r.id.as_deref().ok_or("Run ID required")?;
                let run = lock(&self.inner.runs)?
                    .iter()
                    .find(|x| x.id == id)
                    .cloned()
                    .ok_or("Unknown run")?;
                if matches!(r.action, Action::Stop) {
                    run.stop();
                }
                let code = run.done();
                let out = lock(&run.output)?;
                let from = r.offset.unwrap_or(0).min(out.len());
                use base64::Engine;
                Ok(json!({
                    "data": base64::engine::general_purpose::STANDARD.encode(&out[from..]),
                    "offset": out.len(),
                    "done": code.is_some(),
                    "code": code,
                    "card": run.card,
                    "line": run.line,
                }))
            }
            Action::Suggest => {
                let facts = self.facts()?;
                let started = std::time::Instant::now();
                let (cards, source, note) = match advisor::ask(
                    r.provider.as_deref().unwrap_or(""),
                    r.settings.as_ref(),
                    &advisor::suggest_prompt(),
                    &advisor::compact(&facts).to_string(),
                ) {
                    Ok((engine, text)) => match advisor::parse_suggestions(&facts, &text) {
                        Some(cards) => (cards, engine, None),
                        None => (
                            advisor::rules(&facts),
                            "rules".into(),
                            Some(format!("{engine} replied without usable cards")),
                        ),
                    },
                    Err(e) => (advisor::rules(&facts), "rules".into(), Some(e)),
                };
                lock(&self.inner.cards)?.retain(|c| !c.id.starts_with('s'));
                let cards = cards
                    .into_iter()
                    .map(|c| self.register("s", c))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(
                    json!({"suggestions": cards, "source": source, "note": note, "seconds": started.elapsed().as_secs(), "home": home}),
                )
            }
            Action::Chat => {
                let text = r.text.as_deref().map(str::trim).unwrap_or("");
                if text.is_empty() || text.len() > 2000 {
                    return Err("Ask something short".into());
                }
                let facts = self.facts()?;
                let shown: Vec<Value> = lock(&self.inner.cards)?
                    .iter()
                    .map(|c| json!({"issue": c.issue, "action": c.action, "paths": c.paths.iter().map(|p| facts.tilde(p)).collect::<Vec<_>>()}))
                    .collect();
                let history: Vec<Value> = r
                    .history
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                    .take(8)
                    .rev()
                    .map(|t| json!({"role": t.role, "text": t.text.chars().take(600).collect::<String>()}))
                    .collect();
                let input = json!({"facts": advisor::compact(&facts), "cards_shown": shown, "chat": history, "question": text});
                let (engine, out) = advisor::ask(
                    r.provider.as_deref().unwrap_or(""),
                    r.settings.as_ref(),
                    &advisor::chat_prompt(),
                    &input.to_string(),
                )?;
                let (reply, card) = advisor::parse_chat(&facts, &out)
                    .ok_or(format!("{engine} gave an unreadable answer; try again"))?;
                let card = card.map(|c| self.register("c", c)).transpose()?;
                Ok(json!({"reply": reply, "suggestion": card, "source": engine}))
            }
            Action::Plan => {
                let card = self.card(r.id.as_deref())?;
                let mut paths = Vec::new();
                for p in &card.paths {
                    let path = PathBuf::from(p);
                    let root = card.action == Action2::Delete
                        && space.rel(&path).is_some_and(|rel| {
                            space::CACHE_ROOTS.iter().any(|(c, _)| rel == Path::new(c))
                        });
                    match std::fs::read_dir(&path) {
                        Ok(rd) if root => paths.extend(rd.flatten().map(|e| e.path())),
                        _ => paths.push(path),
                    }
                }
                let rows: Vec<Value> = if card.action == Action2::Command {
                    vec![json!({"path": card.paths[0], "bytes": card.bytes})]
                } else {
                    let sizes = space::freeable_each(&paths);
                    let mut rows: Vec<(PathBuf, u64)> = paths.into_iter().zip(sizes).collect();
                    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    rows.into_iter()
                        .map(|(p, b)| json!({"path": p, "bytes": b, "ok": space.guard(card.action, &p).is_ok()}))
                        .collect()
                };
                Ok(
                    json!({"plan": {"id": card.id, "action": card.action, "rows": rows, "target": space.archive, "command": card.command}, "home": home}),
                )
            }
            Action::Run => {
                let card = self.card(r.id.as_deref())?;
                let paths: Vec<PathBuf> = r
                    .paths
                    .unwrap_or_default()
                    .into_iter()
                    .map(PathBuf::from)
                    .collect();
                // Only the card's own paths, or direct children of a card path (expanded caches).
                if paths.is_empty()
                    || paths.iter().any(|p| {
                        !card
                            .paths
                            .iter()
                            .any(|c| p == Path::new(c) || p.parent() == Some(Path::new(c)))
                    })
                {
                    return Err("Selection does not match this suggestion".into());
                }
                let outcome = match card.action {
                    Action2::Delete => space.delete(&paths),
                    Action2::Move => space.archive(&paths),
                    Action2::Command => return Err("Commands are never run by the app".into()),
                };
                if let Some(facts) = lock(&self.inner.facts)?.as_mut() {
                    facts.items.retain(|i| {
                        i.action == Action2::Command
                            || std::fs::symlink_metadata(&i.path)
                                .is_ok_and(|m| !m.file_type().is_symlink())
                    });
                }
                let archive = space.archive.to_string_lossy().into_owned();
                let to = (card.action == Action2::Move).then_some(archive.as_str());
                self.shift(&card.kind, outcome.freed, Some(&home), to)?;
                if let Some(id) = &outcome.undo {
                    lock(&self.inner.undo_kinds)?.insert(id.clone(), card.kind.clone());
                }
                Ok(
                    json!({"outcome": outcome, "disks": self.disk_view()?, "undo": self.undo_list()?}),
                )
            }
            Action::Undo => {
                let id = r.id.as_deref().ok_or("Undo ID required")?;
                let outcome = space.undo(id);
                let kind = lock(&self.inner.undo_kinds)?
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "other".into());
                let archive = space.archive.to_string_lossy().into_owned();
                self.shift(&kind, outcome.freed, Some(&archive), Some(&home))?;
                Ok(
                    json!({"outcome": outcome, "disks": self.disk_view()?, "undo": self.undo_list()?}),
                )
            }
            _ => Err("Unsupported action".into()),
        }
    }
    /// After an action: re-measure what every open card still points at, drop
    /// paths that are gone or already moved, and drop cards with nothing left.
    /// No rescan; only the card paths are touched.
    fn recheck(&self) -> Result<Value, String> {
        let space = self.space()?;
        let cards = lock(&self.inner.cards)?.clone();
        let mut kept = Vec::new();
        let mut gone = Vec::new();
        let mut live_paths = std::collections::HashSet::new();
        for mut c in cards {
            if c.action == Action2::Command {
                match c.paths.first().and_then(|p| tools::Tool::from_key(p)) {
                    Some(t) => {
                        let bytes = t.probe(&space.home);
                        if bytes < 1 << 20 {
                            gone.push(c.id);
                        } else {
                            c.bytes = bytes;
                            kept.push(c);
                        }
                    }
                    None => kept.push(c),
                }
                continue;
            }
            let paths: Vec<PathBuf> = c.paths.iter().map(PathBuf::from).collect();
            let sizes: Vec<Option<u64>> = match c.action {
                Action2::Delete => paths
                    .iter()
                    .zip(space::freeable_each(&paths))
                    .map(|(p, b)| space.guard(Action2::Delete, p).ok().map(|_| b))
                    .collect(),
                _ => paths.iter().map(|p| placement::live_bytes(p)).collect(),
            };
            let live: Vec<(String, u64)> = c
                .paths
                .iter()
                .cloned()
                .zip(sizes)
                .filter_map(|(p, b)| b.filter(|b| *b > 0).map(|b| (p, b)))
                .collect();
            let bytes: u64 = live.iter().map(|l| l.1).sum();
            if live.is_empty() || bytes < 1 << 20 {
                gone.push(c.id);
                continue;
            }
            c.paths = live.iter().map(|l| l.0.clone()).collect();
            c.bytes = bytes;
            if let Some(g) = c.group {
                if c.id.starts_with('p') {
                    c.issue = g.title_for(&c.paths);
                }
                c.target = Some(placement::target(&space.archive, &c.paths));
            }
            live_paths.extend(live);
            kept.push(c);
        }
        {
            let mut all = lock(&self.inner.cards)?;
            all.retain(|c| !gone.contains(&c.id));
            for k in &kept {
                if let Some(slot) = all.iter_mut().find(|c| c.id == k.id) {
                    *slot = k.clone();
                }
            }
        }
        if let Some(facts) = lock(&self.inner.facts)?.as_mut() {
            facts.items.retain(|i| {
                i.action == Action2::Command
                    || std::fs::symlink_metadata(&i.path).is_ok_and(|m| !m.file_type().is_symlink())
            });
            for i in facts.items.iter_mut() {
                if let Some((_, b)) = live_paths.iter().find(|(p, _)| *p == i.path) {
                    i.bytes = *b;
                }
            }
        }
        Ok(json!({"cards": kept, "gone": gone, "disks": self.disk_view()?}))
    }
    fn card(&self, id: Option<&str>) -> Result<Suggestion, String> {
        let id = id.ok_or("Suggestion ID required")?;
        lock(&self.inner.cards)?
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| "Suggestion expired; ask again".into())
    }
}
const KINDS: [&str; 8] = [
    "video", "images", "code", "deps", "caches", "models", "archives", "other",
];

fn kinds_pending(is_home: bool, complete: bool, running: bool, known: bool) -> bool {
    if is_home {
        !complete
    } else {
        running && !known
    }
}

fn kinds_cache_path() -> Option<PathBuf> {
    let space = Space::from_env().ok()?;
    Some(
        space
            .home
            .join(".local/state/mem-inspector/disk-kinds.json"),
    )
}

fn load_disk_kinds() -> HashMap<String, Value> {
    kinds_cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Measure data types on every other mounted disk, one at a time, in the
/// background. Read-only du; the result is cached for the next launch.
fn scan_other_disks(inner: &Arc<Inner>) {
    {
        let Ok(mut running) = inner.disk_kinds_running.lock() else {
            return;
        };
        if *running {
            return;
        }
        *running = true;
    }
    let inner = inner.clone();
    std::thread::spawn(move || {
        let overview = inner
            .overview
            .lock()
            .ok()
            .and_then(|o| o.clone())
            .unwrap_or(Value::Null);
        let disks = advisor::disks(&overview);
        let home = inner
            .space
            .as_ref()
            .map(|s| s.home.clone())
            .unwrap_or_default();
        let home_mount = disks
            .iter()
            .filter(|d| home.starts_with(&d.mount))
            .max_by_key(|d| d.mount.len())
            .map(|d| d.mount.clone());
        for d in disks {
            if Some(&d.mount) == home_mount.as_ref() {
                continue;
            }
            let out = std::process::Command::new("python3")
                .arg("-c")
                .arg(include_str!("../../scripts/storage-overview.py"))
                .args(["--kinds", &d.mount])
                .stderr(std::process::Stdio::null())
                .output();
            let Some(value) = out
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
            else {
                continue;
            };
            if let Ok(mut deltas) = inner.deltas.lock() {
                deltas.remove(&d.mount);
            }
            if let Ok(mut kinds) = inner.disk_kinds.lock() {
                kinds.insert(d.mount.clone(), value);
                if let Some(path) = kinds_cache_path() {
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    let _ = std::fs::write(&path, serde_json::to_vec(&*kinds).unwrap_or_default());
                }
            }
        }
        if let Ok(mut running) = inner.disk_kinds_running.lock() {
            *running = false;
        }
    });
}

fn aggregate(scan: &Scan) -> mem_ai::ScanSummary {
    // Category names come from a closed enum. Never send paths, names, timestamps or content.
    let mut totals = [(0u64, 0u64); 4];
    for e in scan
        .entries
        .iter()
        .filter(|e| e.kind == "file" && !e.shared)
    {
        let kind = if e.path.split('/').any(|p| matches!(p, ".cache" | "cache")) {
            0
        } else if e
            .path
            .split('/')
            .any(|p| matches!(p, "target" | "node_modules" | "build" | "dist"))
        {
            1
        } else if e.path.split('/').any(|p| p == "Downloads") {
            2
        } else {
            3
        };
        totals[kind].0 = totals[kind].0.saturating_add(e.allocated);
        totals[kind].1 += 1;
    }
    let categories = [
        mem_ai::CategoryKind::Cache,
        mem_ai::CategoryKind::Build,
        mem_ai::CategoryKind::Downloads,
        mem_ai::CategoryKind::Other,
    ]
    .into_iter()
    .zip(totals)
    .map(|(kind, (bytes, file_count))| mem_ai::Category {
        kind,
        bytes,
        file_count,
    })
    .collect();
    mem_ai::ScanSummary {
        total_bytes: scan.allocated,
        file_count: totals.iter().map(|x| x.1).sum(),
        categories,
    }
}
fn backend_eligible(scan: &Scan, path: &str, fixture: Option<&(PathBuf, PathBuf)>) -> bool {
    let trash = match fixture {
        Some((_, t)) => t.clone(),
        None => match home_trash() {
            Ok(t) => t,
            Err(_) => return false,
        },
    };
    let Ok(mut queue) = Queue::new(QueueOptions {
        root: PathBuf::from(&scan.root),
        trash,
        protected_paths: vec![],
        fixture: fixture.is_some(),
    }) else {
        return false;
    };
    // Read-only queue validation also rejects sensitive scopes, symlinks, shared
    // inodes and stale identity. This temporary review never authorizes a move.
    queue.preview(&[path.to_owned()]).is_ok_and(|p| {
        p.items.len() == 1
            && scan
                .entries
                .iter()
                .any(|e| e.path == path && p.items[0] == *e)
    })
}
fn local_evidence(
    scan: &Scan,
    fixture: Option<&(PathBuf, PathBuf)>,
) -> HashMap<String, mem_ai::Evidence> {
    if scan.cancelled || scan.truncated || !scan.errors.is_empty() {
        return HashMap::new();
    }
    scan.entries
        .iter()
        .filter(|e| e.kind == "file" && !e.shared)
        .take(500)
        .filter(|e| {
            crate::evidence::cache_intent(Path::new(&scan.root), Path::new(&e.path))
                && backend_eligible(scan, &e.path, fixture)
        })
        .map(|e| (e.path.clone(), mem_ai::Evidence::CacheIntent))
        .collect()
}
fn home_trash() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .ok_or("Home directory unavailable")?;
    let trash = base.join("Trash");
    if !trash.is_dir() {
        return Err(
            "OS Trash does not exist. Create it through your desktop before cleanup.".into(),
        );
    }
    Ok(trash)
}
#[cfg(target_os = "linux")]
fn storage(root: &Path) -> Value {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let Ok(path) = CString::new(root.as_os_str().as_bytes()) else {
        return Value::Null;
    };
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL terminated and statvfs initializes info on success.
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Value::Null;
    }
    let info = unsafe { info.assume_init() };
    json!({"total":(u128::from(info.f_blocks)*u128::from(info.f_frsize)).to_string(),"free":(u128::from(info.f_bavail)*u128::from(info.f_frsize)).to_string()})
}
#[cfg(not(target_os = "linux"))]
fn storage(_root: &Path) -> Value {
    Value::Null
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn req(v: Value) -> Request {
        serde_json::from_value(v).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, Bridge, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mem-core-fixture-ui");
        let trash = dir.path().join("Trash");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&trash).unwrap();
        std::fs::set_permissions(&trash, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.join("notes.txt"), "fixture text").unwrap();
        let bridge = Bridge::new(Some((root.clone(), trash)));
        (dir, bridge, root)
    }
    #[test]
    fn overview_streams_read_only_and_never_populates_cleanup_queue() {
        let (_dir, bridge, root) = fixture();
        let before = std::fs::read(root.join("notes.txt")).unwrap();
        let start = std::time::Instant::now();
        let first = bridge.request(req(json!({"action":"overview"}))).unwrap();
        assert!(first["pending"].as_bool().unwrap());
        assert!(start.elapsed().as_secs_f64() < 1.0);
        loop {
            let result = bridge.request(req(json!({"action":"overview"}))).unwrap();
            if result["pending"] == false {
                assert_eq!(result["overview"]["complete"], true);
                assert_eq!(result["overview"]["root"], root.to_string_lossy().as_ref());
                assert!(
                    result["overview"]["allocated"]
                        .as_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                        > 0
                );
                break;
            }
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), before);
        assert!(lock(&bridge.inner.data).unwrap().last.is_none());
        assert!(lock(&bridge.inner.data).unwrap().queues.is_empty());
    }
    #[test]
    fn fixture_review_trash_restore_and_replay() {
        let (_dir, b, root) = fixture();
        b.request(req(json!({"action":"scan"}))).unwrap();
        let p = b
            .request(req(
                json!({"action":"preview","paths":[root.join("notes.txt").to_str().unwrap()]}),
            ))
            .unwrap();
        let id = p["preview"]["id"].as_str().unwrap();
        assert!(b
            .request(req(json!({"action":"commit","id":id,"confirmed":false})))
            .is_err());
        assert!(root.join("notes.txt").exists());
        let h = b
            .request(req(json!({"action":"commit","id":id,"confirmed":true})))
            .unwrap();
        assert!(!root.join("notes.txt").exists());
        assert!(b
            .request(req(json!({"action":"commit","id":id,"confirmed":true})))
            .is_err());
        let history_id = h["history"][0]["id"].as_str().unwrap();
        assert!(b
            .request(req(json!({"action":"restore","id":history_id})))
            .is_err());
        b.request(req(
            json!({"action":"restore","id":history_id,"confirmed":true}),
        ))
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "fixture text"
        );
    }
    #[test]
    fn review_changed_file_and_restore_collision_preserve_data() {
        let (_dir, b, root) = fixture();
        b.request(req(json!({"action":"scan"}))).unwrap();
        let path = root.join("notes.txt");
        let p = b
            .request(req(json!({"action":"preview","paths":[path]})))
            .unwrap();
        std::fs::write(&path, "changed fixture").unwrap();
        assert!(b
            .request(req(
                json!({"action":"commit","id":p["preview"]["id"],"confirmed":true})
            ))
            .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed fixture");
        let p = b
            .request(req(json!({"action":"preview","paths":[path]})))
            .unwrap();
        let h = b
            .request(req(
                json!({"action":"commit","id":p["preview"]["id"],"confirmed":true}),
            ))
            .unwrap();
        std::fs::write(&path, "new destination").unwrap();
        assert!(b
            .request(req(
                json!({"action":"restore","id":h["history"][0]["id"],"confirmed":true})
            ))
            .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new destination");
        assert!(Path::new(h["history"][0]["trashed"].as_str().unwrap()).exists());
    }
    #[test]
    fn fixture_scope_and_consent_fail_closed() {
        let (_dir, b, _root) = fixture();
        assert!(b.request(req(json!({"action":"scan","path":"/"}))).is_err());
        assert!(b
            .request(req(
                json!({"action":"advice","provider":"missing","consent":false})
            ))
            .unwrap_err()
            .contains("consent"));
        assert!(
            serde_json::from_value::<Request>(json!({"action":"advice","prompt":"private"}))
                .is_err()
        );
    }
    #[test]
    fn proposal_consent_and_mapping_fail_before_provider() {
        let (_dir, b, _root) = fixture();
        assert!(b
            .request(req(
                json!({"action":"propose","provider":"missing","consent":false})
            ))
            .unwrap_err()
            .contains("consent"));
        b.request(req(json!({"action":"scan"}))).unwrap();
        assert!(b.request(req(json!({"action":"propose","provider":"missing","consent":true,"verified":["/outside/private"]}))).unwrap_err().contains("Unknown confirmed item"));
        lock(&b.inner.data)
            .unwrap()
            .last
            .as_mut()
            .unwrap()
            .truncated = true;
        assert!(b
            .request(req(
                json!({"action":"propose","provider":"missing","consent":true})
            ))
            .unwrap_err()
            .contains("error-free scan"));
    }
    #[test]
    fn automatic_cache_evidence_excludes_sensitive_shared_and_spoofed_items() {
        let (_dir, b, root) = fixture();
        let cache = root.join("declared-cache");
        std::fs::create_dir(&cache).unwrap();
        std::fs::write(
            cache.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        let candidate = cache.join("payload");
        std::fs::write(&candidate, b"synthetic").unwrap();
        std::fs::write(root.join("personal.txt"), b"ambiguous personal data").unwrap();
        let protected = cache.join(".ssh");
        std::fs::create_dir(&protected).unwrap();
        std::fs::write(protected.join("id_rsa"), b"synthetic not a key").unwrap();
        std::os::unix::fs::symlink(&candidate, cache.join("alias")).unwrap();
        let shared = cache.join("shared");
        std::fs::write(&shared, b"shared").unwrap();
        std::fs::hard_link(&shared, cache.join("shared-link")).unwrap();
        let result = b.request(req(json!({"action":"scan"}))).unwrap();
        assert_eq!(result["evidence"].as_object().unwrap().len(), 1);
        assert_eq!(
            result["evidence"][candidate.to_str().unwrap()],
            "cache_intent"
        );
        // A changed snapshot must not inherit eligibility from an earlier scan.
        std::fs::write(&candidate, b"changed length synthetic").unwrap();
        let data = lock(&b.inner.data).unwrap();
        assert!(!backend_eligible(
            data.last.as_ref().unwrap(),
            candidate.to_str().unwrap(),
            b.inner.fixture.as_ref()
        ));
    }
    #[test]
    fn aggregates_never_contain_private_text() {
        let (_dir, b, root) = fixture();
        std::fs::create_dir(root.join("cache")).unwrap();
        std::fs::write(root.join("cache/private-name.txt"), "private contents").unwrap();
        b.request(req(json!({"action":"scan"}))).unwrap();
        let data = lock(&b.inner.data).unwrap();
        let s = aggregate(data.last.as_ref().unwrap());
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("private"));
        assert!(!json.contains(root.to_str().unwrap()));
        assert_eq!(s.file_count, 2);
        assert!(s
            .categories
            .iter()
            .any(|c| matches!(c.kind, mem_ai::CategoryKind::Cache) && c.file_count == 1));
    }
    #[test]
    fn recheck_drops_finished_paths_and_cards_without_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = Space::new(home.clone(), dir.path().join("archive")).unwrap();
        let b = Bridge::with_space(None, Some(space.clone()));
        let a = home.join("Videos/a.mp4");
        let v = home.join("Videos/b.mp4");
        let cache = home.join(".cache/app");
        for (p, n) in [(&a, 3usize << 20), (&v, 2 << 20)] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![1u8; n]).unwrap();
        }
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("blob"), vec![1u8; 2 << 20]).unwrap();
        let card = |action, paths: Vec<&PathBuf>, group| Suggestion {
            id: String::new(),
            issue: "x".into(),
            fix: "y".into(),
            action,
            paths: paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            bytes: 99 << 30,
            command: None,
            kind: "video".into(),
            root: false,
            group,
            target: None,
            tradeoff: None,
        };
        let moved = b
            .register(
                "p",
                card(Action2::Move, vec![&a, &v], Some(placement::Group::Media)),
            )
            .unwrap();
        let cleared = b
            .register("s", card(Action2::Delete, vec![&cache], None))
            .unwrap();
        assert!(space.archive(std::slice::from_ref(&a)).failed.is_empty());
        std::fs::remove_dir_all(&cache).unwrap();
        let r = b.request(req(json!({"action":"recheck"}))).unwrap();
        assert_eq!(r["gone"], json!([cleared.id]));
        let kept = &r["cards"][0];
        assert_eq!(kept["id"], moved.id.as_str());
        assert_eq!(kept["paths"], json!([v.to_string_lossy()]));
        assert_eq!(kept["issue"], "Large video");
        assert!(kept["bytes"].as_u64().unwrap() < 3 << 20);
        assert!(kept["target"].as_str().unwrap().ends_with("/home/Videos"));
        assert_eq!(lock(&b.inner.cards).unwrap().len(), 1);
    }
    #[test]
    fn exec_runs_only_app_commands_after_confirm_and_never_root() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = Space::new(home.clone(), dir.path().join("archive")).unwrap();
        let b = Bridge::with_space(None, Some(space));
        let uv = home.join(".cache/uv/wheels-v5/blob");
        std::fs::create_dir_all(uv.parent().unwrap()).unwrap();
        std::fs::write(&uv, vec![1u8; 2 << 20]).unwrap();
        let card = |key: &str, action| Suggestion {
            id: String::new(),
            issue: "x".into(),
            fix: "y".into(),
            action,
            paths: vec![key.into()],
            bytes: 1,
            command: Some("shown".into()),
            kind: "caches".into(),
            root: false,
            group: None,
            target: None,
            tradeoff: None,
        };
        let root = b
            .register("s", card("system:pacman", Action2::Command))
            .unwrap();
        let bogus = b.register("s", card("/bin/sh", Action2::Command)).unwrap();
        let delete = b
            .register("s", card(home.to_str().unwrap(), Action2::Delete))
            .unwrap();
        let ok = b
            .register("s", card("system:uv", Action2::Command))
            .unwrap();
        let exec = |id: &str, confirmed: bool| {
            b.request(req(json!({"action":"exec","id":id,"confirmed":confirmed})))
        };
        assert!(exec(&root.id, true).unwrap_err().contains("root"));
        assert!(exec(&bogus.id, true).unwrap_err().contains("own cleanup"));
        assert!(exec(&delete.id, true).unwrap_err().contains("own cleanup"));
        assert!(exec(&ok.id, false).unwrap_err().contains("Confirm"));
        if std::process::Command::new("uv")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let started = exec(&ok.id, true).unwrap();
        assert!(started["line"]
            .as_str()
            .unwrap()
            .starts_with("uv cache clean --cache-dir "));
        let run = started["run"].as_str().unwrap().to_string();
        let t = std::time::Instant::now();
        let last = loop {
            let r = b
                .request(req(json!({"action":"term","id":run,"offset":0})))
                .unwrap();
            if r["done"] == true {
                break r;
            }
            assert!(t.elapsed().as_secs() < 30);
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(last["code"], 0, "{last}");
        assert!(!uv.exists(), "uv cleaned the rehearsal cache");
        let r = b.request(req(json!({"action":"recheck"}))).unwrap();
        assert!(r["gone"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g == ok.id.as_str()));
    }
    #[test]
    fn wire_preserves_full_width_integers() {
        let v = wire(json!({"allocated":u64::MAX,"entries":[{"ino":u64::MAX}],"mtime":1234.5}))
            .unwrap();
        assert_eq!(v["allocated"], u64::MAX.to_string());
        assert_eq!(v["entries"][0]["ino"], u64::MAX.to_string());
        assert_eq!(v["mtime"], 1234.5);
    }
    #[test]
    fn cancellation_bypasses_busy_job_lock() {
        let (_dir, b, _root) = fixture();
        let cancel = Cancellation::default();
        *lock(&b.inner.active).unwrap() = Some(cancel.clone());
        assert_eq!(
            b.request(req(json!({"action":"status"}))).unwrap()["fixture"],
            true
        );
        b.request(req(json!({"action":"cancel"}))).unwrap();
        assert!(cancel.is_cancelled());
        assert!(b
            .request(req(json!({"action":"scan"})))
            .unwrap_err()
            .contains("running"));
    }
    #[test]
    #[ignore = "explicit disposable filesystem benchmark"]
    fn benchmark_fixture() {
        let (_dir, b, root) = fixture();
        for i in 0..10000 {
            std::fs::write(root.join(format!("file-{i}.txt")), "fixture bytes").unwrap();
        }
        for run in 0..5 {
            let start = std::time::Instant::now();
            let result = b.request(req(json!({"action":"scan"}))).unwrap();
            println!(
                "BENCH {}",
                json!({"run":run,"entries":result["scan"]["entries"].as_array().unwrap().len(),"scan_and_wire_ms":start.elapsed().as_secs_f64()*1000.0})
            );
        }
    }
}
