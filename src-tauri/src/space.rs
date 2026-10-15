//! Owner-approved mutations: delete rebuildable data, archive cold files with a
//! symlink back, undo archives. Every path passes `Space::guard` immediately
//! before it is touched. Symlinks are never followed.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

/// Rebuildable cache roots, relative to home. Anything inside may be deleted.
pub const CACHE_ROOTS: &[(&str, &str)] = &[
    (".cache", "App caches"),
    (".npm", "npm cache"),
    (".gradle/caches", "Gradle cache"),
    (".gradle/wrapper/dists", "Gradle wrappers"),
    (".yarn/berry/cache", "Yarn cache"),
    (".cargo/registry", "Cargo registry"),
    (".cargo/git", "Cargo git cache"),
    (".bun/install/cache", "Bun cache"),
    (".m2/repository", "Maven cache"),
    (".pnpm-store", "pnpm store"),
    (".local/share/pnpm/store", "pnpm store"),
    (".android/avd", "Android emulators"),
    (".android/cache", "Android cache"),
    ("Android/Sdk/system-images", "Android system images"),
    (".local/share/Trash", "Trash"),
];
/// Dependency or build folders that a package manager recreates.
pub const DEP_NAMES: &[&str] = &["node_modules", ".venv", "venv", "target", "__pycache__"];
/// Never touched, never listed. Relative to home; prefixes match whole components.
const PROTECTED: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".pki",
    ".password-store",
    ".secrets",
    ".aws",
    ".kube",
    ".netrc",
    ".wednesday",
    ".jarvis",
    ".mozilla",
    ".thunderbird",
    ".local/share/keyrings",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/vivaldi",
    ".config/microsoft-edge",
    ".config/gh",
    ".zen",
];
const PROTECTED_PREFIXES: &[&str] = &[".claude", ".codex"];
/// Hidden folders whose big files may still move: model stores, not config.
const MOVE_DOT_ROOTS: &[&str] = &[".ollama/models", ".lmstudio/models"];
/// Cache roots whose content moves rather than gets deleted.
pub const MOVE_CACHE_ROOTS: &[&str] = &["Android/Sdk/system-images"];
/// A folder changed this recently is in use and stays put.
pub const DIR_IDLE_DAYS: u64 = 14;
/// A git project counts as idle after this many days without commits or edits.
pub const PROJECT_IDLE_DAYS: u64 = 60;
/// Keep this much free on the HDD after a move.
const MIN_ARCHIVE_ROOM: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Delete,
    Move,
    Command,
}

#[derive(Debug, Clone)]
pub struct Space {
    pub home: PathBuf,
    pub archive: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Moved {
    pub from: String,
    pub to: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogEntry {
    id: String,
    time: u64,
    #[serde(default)]
    items: Vec<Moved>,
    #[serde(default)]
    undone: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    pub freed: u64,
    pub done: usize,
    pub failed: Vec<String>,
    pub undo: Option<String>,
}

fn clean(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

impl Space {
    /// Home and archive default to $HOME and /mnt/data2/archive. Tests and
    /// rehearsals override both with MEM_INSPECTOR_SCAN_HOME / MEM_INSPECTOR_ARCHIVE.
    pub fn from_env() -> Result<Self, String> {
        let home = std::env::var_os("MEM_INSPECTOR_SCAN_HOME")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .ok_or("Home directory unavailable")?;
        let archive = std::env::var_os("MEM_INSPECTOR_ARCHIVE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/mnt/data2/archive"));
        Self::new(home, archive)
    }
    pub fn new(home: PathBuf, archive: PathBuf) -> Result<Self, String> {
        if !clean(&home) || !clean(&archive) || home == Path::new("/") {
            return Err("Home and archive must be plain absolute paths".into());
        }
        if archive.starts_with(&home) || home.starts_with(&archive) {
            return Err("Archive must live outside home".into());
        }
        Ok(Self { home, archive })
    }
    fn log_path(&self) -> PathBuf {
        self.home.join(".local/state/mem-inspector/moves.jsonl")
    }
    pub fn rel<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.home)
            .ok()
            .filter(|r| r.components().next().is_some())
    }
    pub fn protected(&self, path: &Path) -> bool {
        let Some(rel) = self.rel(path) else {
            return true;
        };
        let first = rel
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        PROTECTED.iter().any(|p| rel.starts_with(p))
            || PROTECTED_PREFIXES.iter().any(|p| first.starts_with(p))
    }
    pub fn cache_root(&self, path: &Path) -> Option<&'static str> {
        let rel = self.rel(path)?;
        CACHE_ROOTS
            .iter()
            .find(|(root, _)| rel.starts_with(root))
            .map(|(_, label)| *label)
    }
    pub fn dep_dir(path: &Path) -> bool {
        match path.file_name().and_then(|n| n.to_str()) {
            Some("target") => path
                .parent()
                .is_some_and(|p| p.join("Cargo.toml").is_file()),
            Some(name) => DEP_NAMES.contains(&name),
            None => false,
        }
    }
    /// The whole safety contract for one path, rechecked right before mutation.
    pub fn guard(&self, kind: Kind, path: &Path) -> Result<fs::Metadata, String> {
        let open = if kind == Kind::Move {
            OpenFiles::scan()
        } else {
            OpenFiles::default()
        };
        self.guard_with(kind, path, &open)
    }

    /// `guard` with a shared snapshot of open files, for checking many paths.
    pub fn guard_with(
        &self,
        kind: Kind,
        path: &Path,
        open: &OpenFiles,
    ) -> Result<fs::Metadata, String> {
        let shown = path.display();
        if kind == Kind::Command {
            return Err("Commands are never executed".into());
        }
        if !clean(path) || self.rel(path).is_none() {
            return Err(format!("{shown}: outside home"));
        }
        if self.protected(path) {
            return Err(format!("{shown}: protected"));
        }
        // No symlinked ancestor may redirect the path outside home.
        let mut walk = self.home.clone();
        for part in self.rel(path).unwrap().components() {
            let meta = fs::symlink_metadata(&walk).map_err(|e| format!("{shown}: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{shown}: reached through a symlink"));
            }
            if part.as_os_str() == ".git" {
                return Err(format!("{shown}: git data"));
            }
            walk.push(part);
        }
        let meta = fs::symlink_metadata(path).map_err(|e| format!("{shown}: {e}"))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{shown}: symlink"));
        }
        match kind {
            Kind::Delete => {
                if self.cache_root(path).is_none() && !Self::dep_dir(path) {
                    return Err(format!("{shown}: not a cache or dependency folder"));
                }
                if meta.is_dir() && path.join(".git").exists() {
                    return Err(format!("{shown}: git repository"));
                }
            }
            Kind::Move => self.guard_move(path, &meta, open)?,
            Kind::Command => unreachable!(),
        }
        Ok(meta)
    }

    /// Moves to the HDD archive: personal files and whole cold folders only.
    /// Never dotfiles or config, git data of active repos, anything open by a
    /// running process, anything owned by someone else or on another disk.
    fn guard_move(&self, path: &Path, meta: &fs::Metadata, open: &OpenFiles) -> Result<(), String> {
        let shown = path.display();
        let rel = self
            .rel(path)
            .ok_or_else(|| format!("{shown}: outside home"))?;
        let dotted = rel
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
        if dotted && !MOVE_DOT_ROOTS.iter().any(|r| rel.starts_with(r)) {
            return Err(format!("{shown}: hidden file or config"));
        }
        let in_deps = rel.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            DEP_NAMES.contains(&c.as_ref()) || c == "site-packages" || c == "vendor"
        }) || rel.starts_with("go/pkg/mod");
        if in_deps {
            return Err(format!(
                "{shown}: dependency or tool cache, delete it instead"
            ));
        }
        if self.cache_root(path).is_some() && !MOVE_CACHE_ROOTS.iter().any(|r| rel.starts_with(r)) {
            return Err(format!("{shown}: cache, delete it instead"));
        }
        if in_repo(&self.home, path) && !(meta.is_file() && git_ignored(path)) {
            return Err(format!("{shown}: tracked by a git project"));
        }
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(format!("{shown}: owned by another user"));
        }
        let home_dev = fs::symlink_metadata(&self.home)
            .map_err(|e| e.to_string())?
            .dev();
        if meta.dev() != home_dev {
            return Err(format!("{shown}: already on another disk"));
        }
        if open.touches(path) {
            return Err(format!("{shown}: open in a running app"));
        }
        if meta.is_dir() {
            let stats = tree_stats(path)?;
            let age = now_nanos() / 1_000_000_000 - stats.newest.max(0) as u64;
            if age < DIR_IDLE_DAYS * 86400 {
                return Err(format!("{shown}: changed in the last {DIR_IDLE_DAYS} days"));
            }
            if path.join(".git").exists() && project_idle_days(path) < PROJECT_IDLE_DAYS {
                return Err(format!("{shown}: active git project"));
            }
        } else if !meta.is_file() {
            return Err(format!("{shown}: not a file or folder"));
        }
        Ok(())
    }

    pub fn delete(&self, paths: &[PathBuf]) -> Outcome {
        let mut out = Outcome {
            freed: freeable(paths),
            ..Default::default()
        };
        let mut lost = Vec::new();
        for path in paths {
            let result = self.guard(Kind::Delete, path).and_then(|meta| {
                if meta.is_dir() {
                    // std's remove_dir_all never follows symlinks (openat + O_NOFOLLOW).
                    fs::remove_dir_all(path)
                } else {
                    fs::remove_file(path)
                }
                .map_err(|e| format!("{}: {e}", path.display()))
            });
            match result {
                Ok(()) => out.done += 1,
                Err(e) => {
                    lost.push(path.clone());
                    out.failed.push(e)
                }
            }
        }
        if !lost.is_empty() {
            // Only report what is really gone.
            let remaining = freeable(
                &lost
                    .iter()
                    .filter(|p| p.exists())
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            out.freed = out.freed.saturating_sub(remaining);
        }
        out
    }

    pub fn archived(&self, path: &Path) -> PathBuf {
        self.archive.join(path.strip_prefix("/").unwrap_or(path))
    }

    /// Copy to the archive, verify sizes and counts, then swap the original
    /// for a symlink. The original is parked (renamed) first so a failure at
    /// any step leaves either the original or a verified copy, never neither.
    pub fn archive(&self, paths: &[PathBuf]) -> Outcome {
        let id = format!("m{}", now_nanos());
        let mut out = Outcome::default();
        let open = OpenFiles::scan();
        let mut checked = Vec::new();
        for path in paths {
            match self.guard_with(Kind::Move, path, &open).and_then(|meta| {
                let stats = if meta.is_dir() {
                    tree_stats(path)?
                } else {
                    TreeStats::file(&meta)
                };
                Ok((path, meta, stats))
            }) {
                Ok(c) => checked.push(c),
                Err(e) => out.failed.push(e),
            }
        }
        progress_start(checked.iter().map(|c| c.2.bytes).sum());
        for (path, meta, stats) in checked {
            let to = self.archived(path);
            let result = (|| {
                if fs::symlink_metadata(&to).is_ok() {
                    return Err(format!("{}: already archived", to.display()));
                }
                let room = free_bytes(&self.archive);
                if room < stats.bytes + MIN_ARCHIVE_ROOM {
                    return Err(format!("{}: not enough room on the HDD", path.display()));
                }
                let partial = suffixed(&to, ".mem-partial");
                if fs::symlink_metadata(&partial).is_ok() {
                    return Err(format!(
                        "{}: unfinished earlier copy, left alone",
                        partial.display()
                    ));
                }
                if meta.is_dir() {
                    copy_verified(path, &partial, &stats)?;
                } else {
                    relocate(path, &partial, &meta)?;
                    progress_add(meta.len());
                }
                fs::rename(&partial, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                let item = Moved {
                    from: path.to_string_lossy().into(),
                    to: to.to_string_lossy().into(),
                    bytes: stats.blocks,
                    dir: meta.is_dir(),
                };
                // Log before the source disappears so undo always knows the pair.
                self.append(&LogEntry {
                    id: id.clone(),
                    time: now_nanos() / 1_000_000_000,
                    items: vec![item.clone()],
                    undone: false,
                })?;
                let parked = sibling(path, "mem-moving");
                fs::rename(path, &parked).map_err(|e| format!("{}: {e}", path.display()))?;
                if let Err(e) = std::os::unix::fs::symlink(&to, path) {
                    let _ = fs::rename(&parked, path);
                    let _ = remove_any(&to);
                    return Err(format!("{}: {e}", path.display()));
                }
                if let Err(e) = remove_any(&parked) {
                    out.failed.push(format!(
                        "{}: moved, leftover not removed: {e}",
                        parked.display()
                    ));
                }
                Ok(item)
            })();
            match result {
                Ok(item) => {
                    out.freed += item.bytes;
                    out.done += 1;
                }
                Err(e) => out.failed.push(e),
            }
        }
        progress_end();
        if out.done > 0 {
            out.undo = Some(id);
        }
        out
    }

    fn append(&self, entry: &LogEntry) -> Result<(), String> {
        let log = self.log_path();
        fs::create_dir_all(log.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| e.to_string())?;
        let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        writeln!(file, "{line}")
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Undo log unavailable: {e}"))
    }

    /// Archive batches that still have at least one live symlink, newest first.
    pub fn undoable(&self) -> Vec<(String, Vec<Moved>)> {
        let text = fs::read_to_string(self.log_path()).unwrap_or_default();
        let mut batches: Vec<(String, Vec<Moved>)> = Vec::new();
        let mut undone = std::collections::HashSet::new();
        for entry in text
            .lines()
            .filter_map(|l| serde_json::from_str::<LogEntry>(l).ok())
        {
            if entry.undone {
                undone.insert(entry.id);
                continue;
            }
            match batches.iter_mut().find(|(id, _)| *id == entry.id) {
                Some((_, items)) => items.extend(entry.items),
                None => batches.push((entry.id, entry.items)),
            }
        }
        batches.retain(|(id, items)| {
            !undone.contains(id)
                && items
                    .iter()
                    .any(|m| fs::read_link(&m.from).is_ok_and(|t| t == Path::new(&m.to)))
        });
        batches.reverse();
        batches
    }

    pub fn undo(&self, id: &str) -> Outcome {
        let mut out = Outcome::default();
        let Some((_, items)) = self.undoable().into_iter().find(|(b, _)| b == id) else {
            out.failed.push("Nothing to undo".into());
            return out;
        };
        progress_start(items.iter().map(|m| m.bytes).sum());
        for m in items {
            let (from, to) = (PathBuf::from(&m.from), PathBuf::from(&m.to));
            let result = (|| {
                if fs::read_link(&from).map_err(|e| e.to_string())? != to {
                    return Err(format!("{}: link changed, left alone", m.from));
                }
                let meta = fs::symlink_metadata(&to).map_err(|e| e.to_string())?;
                if !(meta.is_file() || meta.is_dir()) || !to.starts_with(&self.archive) {
                    return Err(format!("{}: archive copy missing", m.to));
                }
                let parked = sibling(&from, "mem-undo");
                if meta.is_dir() {
                    copy_verified(&to, &parked, &tree_stats(&to)?)?;
                } else {
                    relocate(&to, &parked, &meta)?;
                    progress_add(meta.len());
                }
                fs::remove_file(&from).map_err(|e| e.to_string())?;
                if let Err(e) = fs::rename(&parked, &from) {
                    let _ = std::os::unix::fs::symlink(&to, &from);
                    return Err(format!("{}: {e}", m.from));
                }
                remove_any(&to).map_err(|e| e.to_string())
            })();
            match result {
                Ok(()) => {
                    out.done += 1;
                    out.freed += m.bytes
                }
                Err(e) => out.failed.push(e),
            }
        }
        progress_end();
        let _ = self.append(&LogEntry {
            id: id.into(),
            time: now_nanos() / 1_000_000_000,
            items: vec![],
            undone: true,
        });
        out
    }
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Paths held by running processes (open files, mapped files, working
/// directories), read from /proc like lsof does. Only our own processes are
/// readable, which is what matters for files in our home.
#[derive(Debug, Default)]
pub struct OpenFiles(Vec<PathBuf>);

impl OpenFiles {
    pub fn scan() -> Self {
        let mut found = std::collections::HashSet::new();
        let me = std::process::id().to_string();
        for proc_dir in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let pid = proc_dir.file_name();
            let pid = pid.to_string_lossy();
            if !pid.bytes().all(|b| b.is_ascii_digit()) || pid == me {
                continue;
            }
            let base = proc_dir.path();
            for link in ["cwd", "exe"] {
                if let Ok(t) = fs::read_link(base.join(link)) {
                    found.insert(t);
                }
            }
            for fd in fs::read_dir(base.join("fd"))
                .into_iter()
                .flatten()
                .flatten()
            {
                if let Ok(t) = fs::read_link(fd.path()) {
                    if t.is_absolute() {
                        found.insert(t);
                    }
                }
            }
            if let Ok(maps) = fs::read_to_string(base.join("maps")) {
                for line in maps.lines() {
                    if let Some(i) = line.find(" /") {
                        found.insert(PathBuf::from(line[i + 1..].trim_end_matches(" (deleted)")));
                    }
                }
            }
        }
        Self(found.into_iter().collect())
    }
    /// True when the path, or anything inside it, is in use.
    pub fn touches(&self, path: &Path) -> bool {
        self.0.iter().any(|p| p.starts_with(path))
    }
    #[cfg(test)]
    pub fn with(paths: Vec<PathBuf>) -> Self {
        Self(paths)
    }
}

/// What a tree holds; a copy must match it exactly before the original goes.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TreeStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    /// Apparent bytes of regular files, compared after the copy.
    pub bytes: u64,
    /// Allocated bytes, what the SSD gets back.
    pub blocks: u64,
    pub newest: i64,
}

impl TreeStats {
    fn file(meta: &fs::Metadata) -> Self {
        Self {
            files: 1,
            bytes: meta.len(),
            blocks: meta.blocks() * 512,
            newest: meta.mtime(),
            ..Default::default()
        }
    }
}

/// Walks a folder without following symlinks. Refuses anything a plain copy
/// cannot reproduce: device files, sockets, other owners, nested mounts.
pub fn tree_stats(root: &Path) -> Result<TreeStats, String> {
    let root_meta = fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let uid = unsafe { libc::geteuid() };
    let mut stats = TreeStats::default();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let shown = path.display();
        if meta.dev() != root_meta.dev() {
            return Err(format!("{shown}: another disk is mounted inside"));
        }
        if meta.uid() != uid {
            return Err(format!("{shown}: owned by another user"));
        }
        stats.newest = stats.newest.max(meta.mtime());
        let ft = meta.file_type();
        if ft.is_symlink() {
            stats.links += 1;
        } else if ft.is_dir() {
            stats.dirs += 1;
            for e in fs::read_dir(&path).map_err(|e| format!("{shown}: {e}"))? {
                stack.push(e.map_err(|e| format!("{shown}: {e}"))?.path());
            }
        } else if ft.is_file() {
            stats.files += 1;
            stats.bytes += meta.len();
            if meta.nlink() < 2 || seen.insert(meta.ino()) {
                stats.blocks += meta.blocks() * 512;
            }
        } else {
            return Err(format!("{shown}: special file"));
        }
    }
    Ok(stats)
}

/// Copy a folder, flush it, and prove the copy matches before anyone relies on it.
fn copy_verified(from: &Path, to: &Path, expected: &TreeStats) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    if fs::symlink_metadata(to).is_ok() {
        return Err(format!("{}: already exists, left alone", to.display()));
    }
    let result = copy_tree(from, to).and_then(|_| {
        sync_fs(to);
        let got = tree_stats(to)?;
        if (got.files, got.dirs, got.links, got.bytes)
            != (
                expected.files,
                expected.dirs,
                expected.links,
                expected.bytes,
            )
        {
            return Err(format!(
                "{}: copy does not match ({} of {} files, {} of {} bytes)",
                from.display(),
                got.files,
                expected.files,
                got.bytes,
                expected.bytes
            ));
        }
        Ok(())
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(to);
    }
    result
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let meta = fs::symlink_metadata(from).map_err(|e| err(from, e))?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        let target = fs::read_link(from).map_err(|e| err(from, e))?;
        std::os::unix::fs::symlink(target, to).map_err(|e| err(to, e))?;
        return Ok(());
    }
    if ft.is_dir() {
        fs::create_dir(to).map_err(|e| err(to, e))?;
        for e in fs::read_dir(from).map_err(|e| err(from, e))? {
            let e = e.map_err(|e| err(from, e))?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        let _ = fs::set_permissions(to, fs::Permissions::from_mode(meta.mode() & 0o7777));
        set_times(to, &meta);
        return Ok(());
    }
    if !ft.is_file() {
        return Err(format!("{}: special file", from.display()));
    }
    copy_file(from, to, &meta)
}

/// Byte copy with progress, then size check, mode and times.
fn copy_file(from: &Path, to: &Path, meta: &fs::Metadata) -> Result<(), String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let err = |e: std::io::Error| format!("{}: {e}", from.display());
    let mut src = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(from)
        .map_err(err)?;
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(err)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut copied = 0u64;
    loop {
        let n = src.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(err)?;
        copied += n as u64;
        progress_add(n as u64);
    }
    if copied != meta.len() {
        let _ = fs::remove_file(to);
        return Err(format!("{}: copy size mismatch", from.display()));
    }
    let _ = fs::set_permissions(to, fs::Permissions::from_mode(meta.mode() & 0o7777));
    set_times(to, meta);
    Ok(())
}

fn set_times(path: &Path, meta: &fs::Metadata) {
    let times = [
        libc::timespec {
            tv_sec: meta.atime(),
            tv_nsec: meta.atime_nsec(),
        },
        libc::timespec {
            tv_sec: meta.mtime(),
            tv_nsec: meta.mtime_nsec(),
        },
    ];
    if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        unsafe {
            libc::utimensat(
                libc::AT_FDCWD,
                c.as_ptr(),
                times.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
    }
}

/// Flush the whole filesystem that holds `path` (one syncfs, not one fsync per file).
fn sync_fs(path: &Path) {
    if let Ok(f) = fs::File::open(path) {
        use std::os::fd::AsRawFd;
        unsafe { libc::syncfs(f.as_raw_fd()) };
    }
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// A hidden sibling used while swapping a path for a link or back.
fn sibling(path: &Path, tag: &str) -> PathBuf {
    path.with_file_name(format!(
        ".{}.{tag}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Free bytes on the filesystem holding `path` or its nearest existing parent.
pub fn free_bytes(path: &Path) -> u64 {
    let Some(existing) = path.ancestors().find(|a| a.exists()) else {
        return 0;
    };
    let Ok(c) = std::ffi::CString::new(existing.as_os_str().as_encoded_bytes()) else {
        return 0;
    };
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    st.f_bavail as u64 * st.f_frsize as u64
}

static PROGRESS_DONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROGRESS_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn progress_start(total: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    PROGRESS_DONE.store(0, Relaxed);
    PROGRESS_TOTAL.store(total, Relaxed);
}
fn progress_add(n: u64) {
    PROGRESS_DONE.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}
fn progress_end() {
    PROGRESS_TOTAL.store(0, std::sync::atomic::Ordering::Relaxed);
}
/// Bytes copied so far and the total of the running move; total 0 when idle.
pub fn progress() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (PROGRESS_DONE.load(Relaxed), PROGRESS_TOTAL.load(Relaxed))
}

/// Days since a project last changed: git activity and files, ignoring
/// dependency folders.
pub fn project_idle_days(project: &Path) -> u64 {
    let mut newest = 0i64;
    for extra in [
        ".git/index",
        ".git/logs/HEAD",
        ".git/FETCH_HEAD",
        ".git/HEAD",
    ] {
        if let Ok(m) = fs::symlink_metadata(project.join(extra)) {
            newest = newest.max(m.mtime());
        }
    }
    let mut stack = vec![(project.to_path_buf(), 0)];
    let mut budget = 5000;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            budget -= 1;
            if budget == 0 {
                break;
            }
            let name = e.file_name();
            let name = name.to_string_lossy();
            if DEP_NAMES.contains(&name.as_ref()) || name == ".git" {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            newest = newest.max(m.mtime());
            if m.is_dir() && depth < 3 {
                stack.push((e.path(), depth + 1));
            }
        }
    }
    (now_nanos() / 1_000_000_000).saturating_sub(newest.max(0) as u64) / 86400
}

/// Untracked and ignored by its repository, so moving it never changes git state.
fn git_ignored(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let ignored = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["check-ignore", "-q", "--"])
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let tracked = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ignored && !tracked
}

fn in_repo(home: &Path, path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .take_while(|a| a.starts_with(home) && *a != home)
        .any(|a| a.join(".git").exists())
}

/// Same-filesystem rename, else verified copy + fsync. The source stays until
/// the caller removes it.
fn relocate(from: &Path, to: &Path, meta: &fs::Metadata) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", from.display());
    fs::create_dir_all(to.parent().ok_or("No parent")?).map_err(err)?;
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    use std::os::unix::fs::OpenOptionsExt;
    let mut src = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(from)
        .map_err(err)?;
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(err)?;
    let copied = std::io::copy(&mut src, &mut dst)
        .and_then(|n| dst.sync_all().map(|_| n))
        .map_err(err);
    if copied.as_ref().ok() != Some(&meta.len()) {
        let _ = fs::remove_file(to);
        return Err(copied.err().unwrap_or_else(|| "copy size mismatch".into()));
    }
    let _ = fs::set_permissions(to, fs::Permissions::from_mode(meta.mode() & 0o7777));
    let times = [
        libc::timespec {
            tv_sec: meta.atime(),
            tv_nsec: meta.atime_nsec(),
        },
        libc::timespec {
            tv_sec: meta.mtime(),
            tv_nsec: meta.mtime_nsec(),
        },
    ];
    let c = std::ffi::CString::new(to.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), 0) };
    Ok(())
}

/// Bytes the filesystem really gets back if every path is removed: an inode
/// counts only when all its hardlinks are inside the selection.
pub fn freeable(paths: &[PathBuf]) -> u64 {
    let mut seen: HashMap<(u64, u64), (u64, u64, u64)> = HashMap::new();
    let mut total = 0;
    let mut stack: Vec<PathBuf> = paths.to_vec();
    let mut root_dev = HashMap::new();
    for p in paths {
        if let Ok(m) = fs::symlink_metadata(p) {
            root_dev.insert(p.clone(), m.dev());
        }
    }
    let devs: std::collections::HashSet<u64> = root_dev.values().copied().collect();
    while let Some(path) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !devs.contains(&meta.dev()) {
            continue;
        }
        let blocks = meta.blocks() * 512;
        if meta.nlink() > 1 && !meta.is_dir() {
            let e = seen
                .entry((meta.dev(), meta.ino()))
                .or_insert((0, meta.nlink(), blocks));
            e.0 += 1;
        } else {
            total += blocks;
        }
        if meta.is_dir() {
            if let Ok(rd) = fs::read_dir(&path) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        }
    }
    total
        + seen
            .values()
            .filter(|(count, links, _)| count >= links)
            .map(|(_, _, b)| b)
            .sum::<u64>()
}

/// Per-path freeable bytes, measured in parallel.
pub fn freeable_each(paths: &[PathBuf]) -> Vec<u64> {
    let chunk = paths.len().div_ceil(8).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|p| freeable(std::slice::from_ref(p)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Space) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let archive = dir.path().join("archive");
        fs::create_dir_all(&home).unwrap();
        let space = Space::new(home, archive).unwrap();
        (dir, space)
    }
    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7u8; bytes]).unwrap();
    }

    #[test]
    fn guards_refuse_protected_outside_symlink_git_and_unknown() {
        let (dir, s) = setup();
        let h = &s.home;
        for p in [
            ".ssh/x",
            ".gnupg/x",
            ".claude-work/x",
            ".codex-2/x",
            ".wednesday/x",
            ".config/google-chrome/x",
            ".local/share/keyrings/x",
        ] {
            write(&h.join(p), 1);
            assert!(s.guard(Kind::Delete, &h.join(p)).is_err(), "{p}");
            assert!(s.guard(Kind::Move, &h.join(p)).is_err(), "{p}");
        }
        let outside = dir.path().join("outside");
        write(&outside.join("node_modules/x"), 1);
        assert!(s
            .guard(Kind::Delete, &outside.join("node_modules"))
            .is_err());
        assert!(s
            .guard(Kind::Delete, &h.join("../outside/node_modules"))
            .is_err());
        // Symlinked ancestor into another tree.
        std::os::unix::fs::symlink(&outside, h.join("link")).unwrap();
        assert!(s.guard(Kind::Delete, &h.join("link/node_modules")).is_err());
        // Symlink itself.
        std::os::unix::fs::symlink(&outside, h.join(".cache")).unwrap();
        assert!(s.guard(Kind::Delete, &h.join(".cache")).is_err());
        // Plain source folders are not deletable; git repos never.
        write(&h.join("proj/src/main.rs"), 1);
        assert!(s.guard(Kind::Delete, &h.join("proj/src")).is_err());
        write(&h.join("proj/node_modules/.git/HEAD"), 1);
        assert!(s.guard(Kind::Delete, &h.join("proj/node_modules")).is_err());
        assert!(s
            .guard(Kind::Delete, &h.join("proj/node_modules/.git"))
            .is_err());
        write(&h.join("repo/.git/HEAD"), 1);
        write(&h.join("repo/big.mp4"), 1);
        assert!(s.guard(Kind::Move, &h.join("repo/big.mp4")).is_err());
        // target only counts as build output next to Cargo.toml.
        write(&h.join("notes/target/x"), 1);
        assert!(s.guard(Kind::Delete, &h.join("notes/target")).is_err());
        write(&h.join("crate/Cargo.toml"), 1);
        write(&h.join("crate/target/x"), 1);
        assert!(s.guard(Kind::Delete, &h.join("crate/target")).is_ok());
        assert!(s.guard(Kind::Command, &h.join("crate/target")).is_err());
        assert!(s.guard(Kind::Delete, &s.home).is_err());
        assert!(Space::new(s.home.clone(), s.home.join("archive")).is_err());
    }

    #[test]
    fn delete_removes_for_real_without_following_symlinks() {
        let (dir, s) = setup();
        let keep = dir.path().join("keep/precious");
        write(&keep, 4096);
        let cache = s.home.join(".cache/app");
        write(&cache.join("blob"), 1 << 20);
        std::os::unix::fs::symlink(keep.parent().unwrap(), cache.join("escape")).unwrap();
        let out = s.delete(std::slice::from_ref(&cache));
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert!(!cache.exists());
        assert!(keep.exists(), "symlink target must survive");
        assert!(out.freed >= 1 << 20);
        assert!(!s.home.join(".local/share/Trash").exists(), "not trashed");
    }

    #[test]
    fn freeable_counts_hardlinks_only_when_fully_selected() {
        let (_dir, s) = setup();
        let a = s.home.join(".cache/uv/blob");
        write(&a, 1 << 20);
        let b = s.home.join("proj/.venv/blob");
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::hard_link(&a, &b).unwrap();
        let uv = s.home.join(".cache/uv");
        let venv = s.home.join("proj/.venv");
        assert!(freeable(std::slice::from_ref(&uv)) < 1 << 20);
        assert!(freeable(&[uv, venv]) >= 1 << 20);
    }

    #[test]
    fn move_symlinks_back_logs_and_undoes() {
        let (_dir, s) = setup();
        let video = s.home.join("Videos/old.mkv");
        write(&video, 300_000);
        let before = fs::metadata(&video).unwrap().modified().unwrap();
        let out = s.archive(std::slice::from_ref(&video));
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        let target = s.archive.join(video.strip_prefix("/").unwrap());
        assert_eq!(fs::read_link(&video).unwrap(), target);
        assert_eq!(
            fs::read(&video).unwrap().len(),
            300_000,
            "readable through link"
        );
        assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), before);
        let id = out.undo.unwrap();
        // Survives a restart: a fresh Space reads the persistent log.
        let again = Space::new(s.home.clone(), s.archive.clone()).unwrap();
        assert_eq!(again.undoable()[0].0, id);
        let back = again.undo(&id);
        assert!(back.failed.is_empty(), "{:?}", back.failed);
        let meta = fs::symlink_metadata(&video).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.len(), 300_000);
        assert!(!target.exists());
        assert!(again.undoable().is_empty());
        assert!(!again.undo(&id).failed.is_empty(), "second undo is a no-op");
    }

    #[test]
    fn cross_device_move_copies_verifies_and_undoes() {
        // /tmp and /dev/shm are separate mounts on Linux; skip where they are not.
        let (Ok(home_dir), Ok(archive_dir)) =
            (tempfile::tempdir(), tempfile::tempdir_in("/dev/shm"))
        else {
            return;
        };
        if fs::metadata(home_dir.path()).unwrap().dev()
            == fs::metadata(archive_dir.path()).unwrap().dev()
        {
            return;
        }
        let s = Space::new(home_dir.path().join("h"), archive_dir.path().join("a")).unwrap();
        let file = s.home.join("Videos/x.mp4");
        write(&file, 70_000);
        let out = s.archive(std::slice::from_ref(&file));
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert!(fs::symlink_metadata(&file)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&file).unwrap(), vec![7u8; 70_000]);
        assert!(s.undo(&out.undo.unwrap()).failed.is_empty());
        assert!(fs::symlink_metadata(&file).unwrap().is_file());
        assert!(!s.archived(&file).exists());
    }

    fn age_tree(root: &Path, days: i64) {
        let t = libc::timespec {
            tv_sec: (now_nanos() / 1_000_000_000) as i64 - days * 86400,
            tv_nsec: 0,
        };
        let mut stack = vec![root.to_path_buf()];
        while let Some(p) = stack.pop() {
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
            let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
            unsafe {
                libc::utimensat(
                    libc::AT_FDCWD,
                    c.as_ptr(),
                    [t, t].as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
        }
    }

    fn folder_move_round_trip(s: &Space) {
        let dir = s.home.join("Downloads/project-assets");
        write(&dir.join("a.bin"), 200_000);
        write(&dir.join("nested/deep/b.txt"), 33);
        fs::create_dir_all(dir.join("empty")).unwrap();
        std::os::unix::fs::symlink("nested/deep/b.txt", dir.join("link")).unwrap();
        age_tree(&dir, 40);
        let before = tree_stats(&dir).unwrap();
        assert_eq!((before.files, before.links), (2, 1));
        let out = s.archive(std::slice::from_ref(&dir));
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert_eq!(out.done, 1);
        let target = s.archived(&dir);
        assert_eq!(fs::read_link(&dir).unwrap(), target);
        assert_eq!(
            fs::read(dir.join("nested/deep/b.txt")).unwrap().len(),
            33,
            "readable through link"
        );
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            Path::new("nested/deep/b.txt")
        );
        let after = tree_stats(&target).unwrap();
        assert_eq!(
            (after.files, after.dirs, after.links, after.bytes),
            (before.files, before.dirs, before.links, before.bytes)
        );
        assert!(
            !s.home.join("Downloads/.project-assets.mem-moving").exists(),
            "parked copy removed"
        );
        let logged = &s.undoable()[0].1[0];
        assert!(logged.dir);
        let back = s.undo(&out.undo.unwrap());
        assert!(back.failed.is_empty(), "{:?}", back.failed);
        let meta = fs::symlink_metadata(&dir).unwrap();
        assert!(meta.is_dir(), "real folder again");
        assert_eq!(tree_stats(&dir).unwrap().bytes, before.bytes);
        assert!(!target.exists());
        assert!(!s.home.join("Downloads/.project-assets.mem-undo").exists());
    }

    #[test]
    fn folder_move_copies_verifies_links_back_and_undoes() {
        let (_dir, s) = setup();
        folder_move_round_trip(&s);
    }

    #[test]
    fn cross_device_folder_move_round_trip() {
        let (Ok(home_dir), Ok(archive_dir)) =
            (tempfile::tempdir(), tempfile::tempdir_in("/dev/shm"))
        else {
            return;
        };
        if fs::metadata(home_dir.path()).unwrap().dev()
            == fs::metadata(archive_dir.path()).unwrap().dev()
        {
            return;
        }
        let s = Space::new(home_dir.path().join("h"), archive_dir.path().join("a")).unwrap();
        fs::create_dir_all(&s.home).unwrap();
        folder_move_round_trip(&s);
    }

    #[test]
    fn moves_refuse_dotfiles_recent_folders_special_files_and_tracked_git() {
        let (_dir, s) = setup();
        let h = &s.home;
        write(&h.join(".config/app/big.mkv"), 10);
        write(&h.join(".local/share/x.iso"), 10);
        write(&h.join("Videos/.hidden.mp4"), 10);
        for p in [
            ".config/app/big.mkv",
            ".local/share/x.iso",
            "Videos/.hidden.mp4",
        ] {
            assert!(
                s.guard(Kind::Move, &h.join(p))
                    .unwrap_err()
                    .contains("hidden"),
                "{p}"
            );
        }
        write(&h.join(".ollama/models/blobs/sha256-1"), 10);
        assert!(s
            .guard(Kind::Move, &h.join(".ollama/models/blobs/sha256-1"))
            .is_ok());
        write(&h.join("Downloads/fresh/a.txt"), 10);
        assert!(s
            .guard(Kind::Move, &h.join("Downloads/fresh"))
            .unwrap_err()
            .contains("days"));
        let fifo = h.join("Downloads/odd/pipe");
        fs::create_dir_all(fifo.parent().unwrap()).unwrap();
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
        age_tree(&h.join("Downloads/odd"), 40);
        assert!(s
            .guard(Kind::Move, &h.join("Downloads/odd"))
            .unwrap_err()
            .contains("special"));
        // A git project changed recently stays; an idle one may move whole.
        write(&h.join("code/live/src.rs"), 10);
        fs::create_dir_all(h.join("code/live/.git")).unwrap();
        write(&h.join("code/live/.git/HEAD"), 10);
        age_tree(&h.join("code/live"), 30);
        assert!(s
            .guard(Kind::Move, &h.join("code/live"))
            .unwrap_err()
            .contains("active git"));
        assert!(s.guard(Kind::Move, &h.join("code/live/.git")).is_err());
        assert!(s
            .guard(Kind::Move, &h.join("code/live/src.rs"))
            .unwrap_err()
            .contains("git"));
        age_tree(&h.join("code/live"), 90);
        assert!(s.guard(Kind::Move, &h.join("code/live")).is_ok());
        // Cache content is deleted, not moved, except Android system images.
        write(&h.join("go/pkg/mod/cache/download/x/@v/v1.zip"), 10);
        write(&h.join("app/node_modules/big/video.mp4"), 10);
        for p in [
            "go/pkg/mod/cache/download/x/@v/v1.zip",
            "app/node_modules/big/video.mp4",
        ] {
            assert!(
                s.guard(Kind::Move, &h.join(p))
                    .unwrap_err()
                    .contains("dependency"),
                "{p}"
            );
        }
        write(&h.join(".cache/blob.mp4"), 10);
        assert!(s.guard(Kind::Move, &h.join(".cache/blob.mp4")).is_err());
        write(
            &h.join("Android/Sdk/system-images/android-34/google_apis/x86_64/system.img"),
            10,
        );
        age_tree(&h.join("Android"), 40);
        assert!(s
            .guard(
                Kind::Move,
                &h.join("Android/Sdk/system-images/android-34/google_apis/x86_64")
            )
            .is_ok());
    }

    #[test]
    fn move_fails_cleanly_when_archive_copy_exists() {
        let (_dir, s) = setup();
        let dir = s.home.join("Downloads/old");
        write(&dir.join("a"), 10);
        age_tree(&dir, 40);
        write(&s.archived(&dir).join("other"), 1);
        let out = s.archive(std::slice::from_ref(&dir));
        assert_eq!(out.done, 0);
        assert!(out.failed[0].contains("already archived"));
        assert!(
            fs::symlink_metadata(&dir).unwrap().is_dir(),
            "original untouched"
        );
        assert_eq!(progress().1, 0, "progress reset");
    }

    #[test]
    fn undo_leaves_changed_links_alone_and_move_refuses_collisions() {
        let (_dir, s) = setup();
        let a = s.home.join("Downloads/a.iso");
        write(&a, 10);
        let id = s.archive(std::slice::from_ref(&a)).undo.unwrap();
        fs::remove_file(&a).unwrap();
        write(&a, 3);
        assert!(s.undo(&id).done == 0);
        assert_eq!(fs::read(&a).unwrap().len(), 3);
        let b = s.home.join("Downloads/b.iso");
        write(&b, 10);
        write(&s.archived(&b), 1);
        let out = s.archive(std::slice::from_ref(&b));
        assert_eq!(out.done, 0);
        assert!(fs::symlink_metadata(&b).unwrap().is_file());
    }
}
