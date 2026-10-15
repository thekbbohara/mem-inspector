//! Turns the home scan into a compact fact sheet, asks Claude (or Codex) for
//! short suggestion cards, and validates every card against those facts. A
//! rule-based set guarantees the screen is never empty.
use crate::placement::{self, Group};
use crate::space::{freeable_each, Kind, Space, CACHE_ROOTS, MOVE_CACHE_ROOTS};
use crate::tools;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const MIN_CACHE: u64 = 200 << 20;
const MIN_DEPS: u64 = 100 << 20;
const IDLE_DAYS: u64 = 30;
const MIN_TOOL: u64 = 200 << 20;
const MAX_CARDS: usize = 6;

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub path: String,
    pub bytes: u64,
    pub action: Kind,
    pub what: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_days: Option<u64>,
    #[serde(skip)]
    pub command: Option<String>,
    #[serde(skip)]
    pub group: Option<Group>,
}

impl Item {
    /// Data type, for colour.
    pub fn kind(&self) -> &'static str {
        match (self.action, self.group) {
            (_, Some(g)) => g.kind(),
            (Kind::Command, _) => tools::Tool::from_key(&self.path).map_or("other", |t| t.kind()),
            _ if self.idle_days.is_some() => "deps",
            _ => "caches",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Disk {
    pub name: String,
    pub mount: String,
    pub medium: String,
    pub total: u64,
    pub free: u64,
    pub used: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Facts {
    /// The scan these facts came from (unix seconds).
    pub scanned_at: i64,
    pub home: String,
    pub archive: String,
    pub disks: Vec<Disk>,
    pub top: Vec<(String, u64)>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    #[serde(default)]
    pub id: String,
    pub issue: String,
    pub fix: String,
    pub action: Kind,
    pub paths: Vec<String>,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub command: Option<String>,
    /// Data type, for colour: video, images, code, deps, caches, models, archives, other.
    #[serde(default)]
    pub kind: String,
    /// Command cards: true when it needs root, so it is shown and never run.
    #[serde(default)]
    pub root: bool,
    /// Placement cards: group, HDD folder and the one-line tradeoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Group>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tradeoff: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Turn {
    pub role: String,
    pub text: String,
}

pub fn human(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 || v >= 10.0 {
        format!("{:.0} {}", v, units[i])
    } else {
        format!("{:.1} {}", v, units[i])
    }
}

#[cfg(test)]
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Facts {
    pub fn tilde(&self, path: &str) -> String {
        match Path::new(path).strip_prefix(&self.home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => path.to_string(),
        }
    }
    fn item(&self, path: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.path == path)
    }
}

/// Live free space for the disks the scan found. /boot is noise for the owner.
pub fn disks(overview: &Value) -> Vec<Disk> {
    let mut out = Vec::new();
    for d in overview["disks"].as_array().into_iter().flatten() {
        let mount = d["mount"].as_str().unwrap_or("");
        if mount.is_empty() || mount.starts_with("/boot") {
            continue;
        }
        let Ok(c) = std::ffi::CString::new(mount) else {
            continue;
        };
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            continue;
        }
        let name = if mount == "/" {
            "System".into()
        } else {
            mount.rsplit('/').next().unwrap_or(mount).to_string()
        };
        let medium = d["medium"].as_str().unwrap_or("");
        out.push(Disk {
            name,
            mount: mount.into(),
            medium: if medium.starts_with("HDD") {
                "HDD"
            } else {
                "SSD"
            }
            .into(),
            total: st.f_blocks as u64 * st.f_frsize as u64,
            free: st.f_bavail as u64 * st.f_frsize as u64,
            used: (st.f_blocks as u64).saturating_sub(st.f_bfree as u64) * st.f_frsize as u64,
        });
    }
    out
}

fn idle_days(project: &Path) -> u64 {
    crate::space::project_idle_days(project)
}

/// Cleanup commands the app can offer, with what each would free. A
/// rehearsal home only gets commands scoped to that home.
fn system_items(home: &Path) -> Vec<Item> {
    let real = std::env::var_os("HOME").is_some_and(|h| Path::new(&h) == home);
    tools::found(home, real)
        .into_iter()
        .filter(|(_, bytes)| *bytes >= MIN_TOOL)
        .map(|(t, bytes)| Item {
            path: t.key().into(),
            bytes,
            action: Kind::Command,
            what: t.what().into(),
            idle_days: None,
            command: Some(t.line(home)),
            group: None,
        })
        .collect()
}

/// Compact, accurate fact sheet. All sizes are bytes really freed (hardlink aware).
pub fn gather(space: &Space, overview: &Value) -> Facts {
    let mut facts = Facts {
        home: space.home.to_string_lossy().into(),
        archive: space
            .archive
            .as_ref()
            .map(|a| a.to_string_lossy().into())
            .unwrap_or_default(),
        scanned_at: overview["scannedAt"].as_i64().unwrap_or(0),
        disks: disks(overview),
        ..Default::default()
    };
    for f in overview["folders"]
        .as_array()
        .into_iter()
        .flatten()
        .take(12)
    {
        let bytes = f["allocated"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if let Some(p) = f["path"].as_str() {
            facts.top.push((facts.tilde(p), bytes));
        }
    }
    let mut targets: Vec<(PathBuf, String, Option<u64>)> = Vec::new();
    for (rel, label) in CACHE_ROOTS {
        let p = space.home.join(rel);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir())
            && !space.protected(&p)
            && !MOVE_CACHE_ROOTS.contains(rel)
        {
            targets.push((p, (*label).into(), None));
        }
    }
    let mut seen = HashSet::new();
    for c in overview["candidates"].as_array().into_iter().flatten() {
        let Some(p) = c["path"].as_str().map(PathBuf::from) else {
            continue;
        };
        if !Space::dep_dir(&p) || space.protected(&p) || space.cache_root(&p).is_some() {
            continue;
        }
        if !seen.insert(p.clone()) {
            continue;
        }
        let days = idle_days(p.parent().unwrap_or(&p));
        if days >= IDLE_DAYS {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            targets.push((
                p,
                format!("{name} in a project idle {days} days"),
                Some(days),
            ));
        }
    }
    let paths: Vec<PathBuf> = targets.iter().map(|t| t.0.clone()).collect();
    let (sizes, places, system) = std::thread::scope(|s| {
        // Without an archive folder there is nowhere to move to: no move facts.
        let cold = s.spawn(|| {
            if space.archive.is_some() {
                placement::find(space, overview)
            } else {
                Vec::new()
            }
        });
        let system = s.spawn(|| system_items(&space.home));
        (
            freeable_each(&paths),
            cold.join().unwrap_or_default(),
            system.join().unwrap_or_default(),
        )
    });
    for ((path, what, days), bytes) in targets.into_iter().zip(sizes) {
        let floor = if days.is_some() { MIN_DEPS } else { MIN_CACHE };
        if bytes >= floor {
            facts.items.push(Item {
                path: path.to_string_lossy().into(),
                bytes,
                action: Kind::Delete,
                what,
                idle_days: days,
                command: None,
                group: None,
            });
        }
    }
    facts.items.extend(places.into_iter().map(|p| Item {
        path: p.path,
        bytes: p.bytes,
        action: Kind::Move,
        what: p.group.what().into(),
        idle_days: p.idle_days,
        command: None,
        group: Some(p.group),
    }));
    facts.items.extend(system);
    facts.items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
    facts
}

/// What Claude sees: tilde paths, human sizes, no noise.
pub fn compact(facts: &Facts) -> Value {
    json!({
        "disks": facts.disks.iter().map(|d| json!({"name": d.name, "mount": d.mount, "type": d.medium, "free": human(d.free), "size": human(d.total), "used_pct": ((d.total.saturating_sub(d.free)) as f64 * 100.0 / d.total.max(1) as f64).round()})).collect::<Vec<_>>(),
        "home_on": facts
            .disks
            .iter()
            .filter(|d| facts.home.starts_with(&d.mount))
            .max_by_key(|d| d.mount.len())
            .map(|d| format!("{} ({})", d.name, d.medium)),
        "archive_target": facts.archive,
        "top_home_folders": facts.top.iter().map(|(p, b)| json!([p, human(*b)])).collect::<Vec<_>>(),
        "items": facts.items.iter().filter(|i| i.action != Kind::Move).map(|i| json!({"path": facts.tilde(&i.path), "size": human(i.bytes), "bytes": i.bytes, "action": i.action, "what": i.what, "idle_days": i.idle_days})).collect::<Vec<_>>(),
        "placement": facts.items.iter().filter(|i| i.action == Kind::Move).map(|i| json!({"path": facts.tilde(&i.path), "size": human(i.bytes), "bytes": i.bytes, "action": i.action, "what": i.what, "idle_days": i.idle_days, "hdd_target": placement::target(Path::new(&facts.archive), std::slice::from_ref(&i.path)), "tradeoff": i.group.map(|g| g.tradeoff())})).collect::<Vec<_>>(),
        "keep_on_ssd": placement::KEEP_ON_SSD,
    })
}

/// Owner-facing card for a set of items, worded without AI.
fn card(issue: String, fix: &str, items: &[&Item]) -> Suggestion {
    Suggestion {
        id: String::new(),
        issue,
        fix: fix.into(),
        action: items[0].action,
        paths: items.iter().map(|i| i.path.clone()).collect(),
        bytes: items.iter().map(|i| i.bytes).sum(),
        command: items[0].command.clone(),
        kind: items[0].kind().into(),
        root: is_root(items[0]),
        group: None,
        target: None,
        tradeoff: None,
    }
}

fn is_root(item: &Item) -> bool {
    tools::Tool::from_key(&item.path).is_some_and(|t| t.needs_root())
}

/// One deterministic card per placement group: what, size, SSD -> HDD target, tradeoff.
pub fn placement_cards(facts: &Facts) -> Vec<Suggestion> {
    let archive = Path::new(&facts.archive);
    let mut out = Vec::new();
    for g in Group::ALL {
        let items: Vec<&Item> = facts.items.iter().filter(|i| i.group == Some(g)).collect();
        if items.is_empty() {
            continue;
        }
        let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
        out.push(Suggestion {
            id: String::new(),
            issue: g.title_for(&paths),
            fix: g.tradeoff().into(),
            action: Kind::Move,
            bytes: items.iter().map(|i| i.bytes).sum(),
            target: Some(placement::target(archive, &paths)),
            paths,
            command: None,
            kind: g.kind().into(),
            root: false,
            group: Some(g),
            tradeoff: Some(g.tradeoff().into()),
        });
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    out
}

pub fn rules(facts: &Facts) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let deps: Vec<&Item> = facts
        .items
        .iter()
        .filter(|i| i.action == Kind::Delete && i.idle_days.is_some())
        .collect();
    for i in facts
        .items
        .iter()
        .filter(|i| i.action == Kind::Delete && i.idle_days.is_none())
    {
        let issue = format!("{} in {}", i.what, facts.tilde(&i.path));
        out.push(card(issue, "Clear it, apps rebuild it", &[i]));
    }
    if !deps.is_empty() {
        let issue = format!("Dependencies in {} projects idle 30+ days", deps.len());
        out.push(card(issue, "Delete them, reinstall when needed", &deps));
    }
    for i in facts.items.iter().filter(|i| i.action == Kind::Command) {
        let mut issue = i.what.replace(" (needs root)", "");
        issue[..1].make_ascii_uppercase();
        let fix = if is_root(i) {
            "Needs root, run it in your own terminal"
        } else {
            "Runs here, output shown live"
        };
        out.push(card(issue, fix, &[i]));
    }
    finish(out)
}

fn finish(mut cards: Vec<Suggestion>) -> Vec<Suggestion> {
    cards.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    cards.truncate(MAX_CARDS);
    cards
}

/// Accept a model card only if every path is a known item with the same action.
fn validate(facts: &Facts, raw: &Value, used: &mut HashSet<String>) -> Option<Suggestion> {
    let action: Kind = serde_json::from_value(raw["action"].clone()).ok()?;
    let text = |k: &str, max: usize| -> Option<String> {
        let s = raw[k].as_str()?.trim().replace('\u{2014}', "-");
        (!s.is_empty() && s.chars().count() <= max && !s.contains('\n')).then_some(s)
    };
    let (issue, fix) = (text("issue", 110)?, text("fix", 70)?);
    let mut items = Vec::new();
    for p in raw["paths"].as_array()? {
        let p = p.as_str()?;
        let full = match p.strip_prefix("~/") {
            Some(rel) => format!("{}/{rel}", facts.home.trim_end_matches('/')),
            None => p.to_string(),
        };
        let item = facts.item(&full)?;
        if item.action != action {
            return None;
        }
        if used.insert(full) {
            items.push(item);
        }
    }
    if items.is_empty() || (action == Kind::Command && items.len() != 1) {
        return None;
    }
    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
    let group = items[0].group;
    Some(Suggestion {
        id: String::new(),
        issue,
        fix,
        action,
        bytes: items.iter().map(|i| i.bytes).sum(),
        command: items[0].command.clone(),
        kind: items[0].kind().into(),
        root: is_root(items[0]),
        target: (action == Kind::Move)
            .then(|| placement::target(Path::new(&facts.archive), &paths)),
        tradeoff: group.map(|g| g.tradeoff().into()),
        group,
        paths,
    })
}

fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

const RULES: &str = "Use only paths from facts.items or facts.placement, copied exactly, with that item's own action. \
Actions: delete = rebuildable cache or dependencies, removed for real. move = data from facts.placement moved from the fast SSD to the slow, roomy HDD (hdd_target) with a symlink left behind, undoable. \
command = a cleanup command from facts.items (system:... path); the app runs it in its own terminal after the owner confirms, or shows it when it needs root; give exactly one path for it. \
Never invent paths, sizes or commands. No markdown, no emoji, no em dash.";

pub fn suggest_prompt() -> String {
    format!(
        "You are the storage assistant inside Mem Inspector, a Linux desktop app. The owner's SSD is nearly full and they want decisions, not reading. \
Input is JSON scan facts. Return ONLY JSON: {{\"suggestions\":[{{\"issue\":\"...\",\"fix\":\"...\",\"action\":\"delete|move|command\",\"paths\":[\"...\"]}}]}}. \
At most 6 suggestions, biggest first, best for freeing the System SSD. Group related items, for example all idle node_modules and .venv folders in one card. Only delete and command cards: the app shows facts.placement moves in its own section. \
issue: one short line under 60 characters naming the thing, like \"~/.cache is rebuildable app cache\". Never put sizes or numbers of bytes in issue or fix: the app shows the exact size next to it. fix: imperative under 40 characters, like \"Clear it\". {RULES}"
    )
}

pub fn chat_prompt() -> String {
    format!(
        "You are the storage assistant inside Mem Inspector, a Linux desktop app. The owner asks about their disks. Input JSON has scan facts, the cards already shown, recent chat and the question. \
Return ONLY JSON: {{\"reply\":\"...\",\"suggestion\":null}} or with suggestion {{\"issue\":\"...\",\"fix\":\"...\",\"action\":\"delete|move|command\",\"paths\":[\"...\"]}}. \
reply: at most two short sentences, under 180 characters in total, direct, numbers over adjectives. Do not list paths in the reply; a card shows them. Add a suggestion only when an action helps and fits the facts. \
If they ask what to move to the HDD or how to use their disks, answer from facts.placement (what, size, tradeoff) and facts.keep_on_ssd, and add one move suggestion with the best placement paths. If placement is empty, say what stays on the SSD and why. \
If nothing in the facts matches what they ask, say so plainly. {RULES}"
    )
}

pub fn parse_suggestions(facts: &Facts, text: &str) -> Option<Vec<Suggestion>> {
    let v = extract_json(text)?;
    let mut used = HashSet::new();
    let cards: Vec<Suggestion> = v["suggestions"]
        .as_array()?
        .iter()
        .filter_map(|raw| validate(facts, raw, &mut used))
        .filter(|c| c.action != Kind::Move)
        .collect();
    (!cards.is_empty()).then(|| finish(cards))
}

pub fn parse_chat(facts: &Facts, text: &str) -> Option<(String, Option<Suggestion>)> {
    let v = extract_json(text)?;
    let reply = v["reply"].as_str()?.trim().replace('\u{2014}', "-");
    if reply.is_empty() || reply.len() > 600 {
        return None;
    }
    let card = if v["suggestion"].is_object() {
        validate(facts, &v["suggestion"], &mut HashSet::new())
    } else {
        None
    };
    Some((reply, card))
}

/// Ask the chosen engine, then Codex if Claude is unavailable.
pub fn ask(
    provider: &str,
    settings: Option<&mem_ai::EngineSettings>,
    system: &str,
    input: &str,
) -> Result<(String, String), String> {
    let mut errors = Vec::new();
    let order: Vec<&str> = match provider {
        "" | "claude" => vec!["claude", "codex"],
        other => vec![other],
    };
    for (n, name) in order.iter().enumerate() {
        let s = if n == 0 { settings } else { None };
        match mem_ai::ask(name, s, system, input) {
            Ok(text) => return Ok((name.to_string(), text)),
            Err(e) => {
                eprintln!("engine {name} failed: {e}");
                errors.push(format!("{name}: {e}"))
            }
        }
    }
    Err(errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts() -> Facts {
        let item = |path: &str, bytes, action, days| Item {
            path: path.into(),
            bytes,
            action,
            what: "x".into(),
            idle_days: days,
            command: (action == Kind::Command).then(|| "sudo paccache -rk1".into()),
            group: (action == Kind::Move).then_some(Group::Media),
        };
        Facts {
            home: "/h".into(),
            archive: "/a".into(),
            items: vec![
                item("/h/.cache", 30 << 30, Kind::Delete, None),
                item("/h/p/node_modules", 5 << 30, Kind::Delete, Some(90)),
                item("/h/q/.venv", 3 << 30, Kind::Delete, Some(40)),
                item("/h/Videos/a.mkv", 2 << 30, Kind::Move, Some(100)),
                item("system:pacman", 1 << 30, Kind::Command, None),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn rules_cover_every_kind_biggest_first() {
        let cards = rules(&facts());
        assert_eq!(cards[0].paths, vec!["/h/.cache"]);
        assert_eq!(cards[0].issue, "x in ~/.cache");
        assert_eq!(cards[1].paths.len(), 2);
        assert_eq!(cards[1].bytes, 8 << 30);
        assert_eq!(cards[1].kind, "deps");
        assert_eq!(cards[2].command.as_deref(), Some("sudo paccache -rk1"));
        assert!(
            cards.iter().all(|c| c.action != Kind::Move),
            "moves live in placement"
        );
        assert!(cards.windows(2).all(|w| w[0].bytes >= w[1].bytes));
    }

    #[test]
    fn placement_cards_show_target_and_tradeoff() {
        let cards = placement_cards(&facts());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].issue, "Large video");
        assert_eq!(cards[0].target.as_deref(), Some("/a/h/Videos"));
        assert_eq!(cards[0].kind, "video");
        assert!(cards[0].tradeoff.as_deref().unwrap().contains("slower"));
        let (_, card) = parse_chat(&facts(), r#"{"reply":"Move the video.","suggestion":{"issue":"Old video","fix":"Move it","action":"move","paths":["~/Videos/a.mkv"]}}"#).unwrap();
        let card = card.unwrap();
        assert_eq!(card.target.as_deref(), Some("/a/h/Videos"));
        assert_eq!(card.group, Some(Group::Media));
    }

    #[test]
    fn model_cards_are_checked_against_facts() {
        let f = facts();
        let text = r#"Sure: ```json
{"suggestions":[
 {"issue":"~/.cache is 30 GB","fix":"Clear it","action":"delete","paths":["~/.cache"],"bytes":999},
 {"issue":"Steal keys","fix":"Delete","action":"delete","paths":["~/.ssh"]},
 {"issue":"Wrong action","fix":"Delete it","action":"delete","paths":["/h/Videos/a.mkv"]},
 {"issue":"Old deps","fix":"Delete","action":"delete","paths":["/h/p/node_modules","/h/q/.venv","~/.cache"]},
 {"issue":"Pacman","fix":"Run it","action":"command","paths":["system:pacman"]}
]}```"#;
        let cards = parse_suggestions(&f, text).unwrap();
        assert_eq!(cards.len(), 3);
        assert_eq!(
            cards[0].bytes,
            30 << 30,
            "bytes come from facts, not the model"
        );
        assert_eq!(
            cards[1].paths,
            vec!["/h/p/node_modules", "/h/q/.venv"],
            "no path reused"
        );
        assert_eq!(cards[2].command.as_deref(), Some("sudo paccache -rk1"));
        assert!(parse_suggestions(&f, "no json").is_none());
        assert!(parse_suggestions(
            &f,
            r#"{"suggestions":[{"issue":"x","fix":"y","action":"move","paths":["/etc/passwd"]}]}"#
        )
        .is_none());
    }

    #[test]
    fn chat_reply_and_optional_card() {
        let f = facts();
        let (reply, card) = parse_chat(&f, r#"{"reply":"Your SSD is mostly caches.","suggestion":{"issue":"Old video","fix":"Move it","action":"move","paths":["~/Videos/a.mkv"]}}"#).unwrap();
        assert_eq!(reply, "Your SSD is mostly caches.");
        assert_eq!(card.unwrap().action, Kind::Move);
        let (_, none) = parse_chat(&f, r#"{"reply":"Nothing to do.","suggestion":null}"#).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn gather_finds_caches_idle_deps_and_cold_files_on_fake_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = Space::new(home.clone(), Some(dir.path().join("archive"))).unwrap();
        let big = |p: &Path, n: usize| {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![1u8; n]).unwrap();
        };
        big(&home.join(".cache/x/blob"), 210 << 20);
        big(&home.join(".ssh/huge.mkv"), 120 << 20);
        let nm = home.join("old/node_modules/pkg/blob");
        big(&nm, 110 << 20);
        let video = home.join("Videos/trip.mkv");
        big(&video, 120 << 20);
        let past = libc::timespec {
            tv_sec: (now() - 90 * 86400) as i64,
            tv_nsec: 0,
        };
        for p in [&video, &home.join("old/package.json")] {
            if !p.exists() {
                big(p, 10);
            }
            let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
            unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), [past, past].as_ptr(), 0) };
        }
        let overview = json!({
            "candidates":[{"path": home.join("old/node_modules").to_str().unwrap()}],
            "bigFiles":[{"path": video, "allocated": (120u64 << 20).to_string(), "mtime": now() - 90 * 86400, "atime": now() - 90 * 86400},
                        {"path": home.join(".ssh/huge.mkv"), "allocated": (120u64 << 20).to_string(), "mtime": 0, "atime": 0}],
        });
        let f = gather(&space, &overview);
        let paths: Vec<_> = f.items.iter().map(|i| f.tilde(&i.path)).collect();
        assert!(paths.contains(&"~/.cache".to_string()), "{paths:?}");
        assert!(
            paths.contains(&"~/Videos/trip.mkv".to_string()),
            "{paths:?}"
        );
        assert!(
            paths.contains(&"~/old/node_modules".to_string()),
            "{paths:?}"
        );
        assert!(!paths.iter().any(|p| p.contains(".ssh")));
        assert!(!rules(&f).is_empty());
    }
}

/// Read-only rehearsal on a real scan: `MEM_OVERVIEW_JSON=scan.json cargo test real_home -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore = "reads the real home and calls the AI engine"]
fn real_home_rehearsal() {
    let path = std::env::var("MEM_OVERVIEW_JSON").expect("MEM_OVERVIEW_JSON");
    let overview: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let space = Space::from_env().unwrap();
    let t = std::time::Instant::now();
    let facts = gather(&space, &overview);
    println!("gather {:?}", t.elapsed());
    println!(
        "{}",
        serde_json::to_string_pretty(&compact(&facts)).unwrap()
    );
    for c in rules(&facts) {
        println!("RULE {} | {} | {}", c.issue, c.fix, human(c.bytes));
    }
    let t = std::time::Instant::now();
    match ask(
        "claude",
        None,
        &suggest_prompt(),
        &compact(&facts).to_string(),
    ) {
        Ok((engine, text)) => {
            println!("{engine} {:?}\n{text}", t.elapsed());
            for c in parse_suggestions(&facts, &text).unwrap_or_default() {
                println!(
                    "AI {} | {} | {} | {:?}",
                    c.issue,
                    c.fix,
                    human(c.bytes),
                    c.action
                );
            }
        }
        Err(e) => println!("AI error {e}"),
    }
}
