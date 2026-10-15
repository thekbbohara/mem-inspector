//! Bounded disk inspection and explicitly reviewed, recoverable Linux Trash moves.
//! See API.md for integration and concurrency limitations.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Filesystem: {0}")]
    Io(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Protected path: {0}")]
    Protected(String),
    #[error("Identity changed: {0}")]
    Changed(String),
    #[error("Resource limit: {0}")]
    Limit(String),
    #[error("Cancelled")]
    Cancelled,
    #[error("Unknown review")]
    UnknownReview,
    #[error("Unknown history")]
    UnknownHistory,
    #[error("Confirmation required")]
    ConfirmationRequired,
    #[error("Unsupported platform")]
    UnsupportedPlatform,
    #[error("Manual recovery required: {original} retained at {retained}")]
    Recovery { original: String, retained: String },
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_entries: usize,
    pub max_errors: usize,
    pub max_depth: usize,
    pub max_path_bytes: usize,
    pub max_hash_bytes: u64,
    pub max_file_hash_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_errors: 1000,
            max_depth: 128,
            max_path_bytes: 4096,
            max_hash_bytes: 1 << 30,
            max_file_hash_bytes: 256 << 20,
        }
    }
}
impl Limits {
    #[cfg(target_os = "linux")]
    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_entries == 0
            || self.max_entries > 100_000
            || self.max_errors == 0
            || self.max_errors > 1000
            || self.max_depth == 0
            || self.max_depth > 128
            || self.max_path_bytes == 0
            || self.max_path_bytes > 4096
            || self.max_hash_bytes == 0
            || self.max_hash_bytes > 8 << 30
            || self.max_file_hash_bytes == 0
            || self.max_file_hash_bytes > 1 << 30
        {
            return Err(Error::Limit("invalid limits or hard cap exceeded".into()));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub apparent: u64,
    pub allocated: u64,
    pub mtime: f64,
    pub dev: u64,
    pub ino: u64,
    pub shared: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanError {
    pub path: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub root: String,
    pub entries: Vec<Entry>,
    pub errors: Vec<ScanError>,
    pub apparent: u64,
    pub allocated: u64,
    pub cancelled: bool,
    pub max_concurrency: usize,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preview {
    pub id: String,
    pub items: Vec<Entry>,
    pub created: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    pub id: String,
    pub original: String,
    pub trashed: String,
    pub info: String,
    pub dev: u64,
    pub ino: u64,
    #[serde(default)]
    pub restored: bool,
}
#[derive(Debug, Clone)]
pub struct QueueOptions {
    pub root: PathBuf,
    pub trash: PathBuf,
    pub protected_paths: Vec<PathBuf>,
    pub fixture: bool,
}

#[cfg(target_os = "linux")]
pub(crate) fn path_text(p: &Path) -> Result<String> {
    let s = p
        .to_str()
        .ok_or_else(|| Error::InvalidPath("non-UTF-8".into()))?;
    if !s.starts_with('/')
        || s.len() > 4096
        || s.contains('\0')
        || (s != "/"
            && s[1..]
                .split('/')
                .any(|c| c.is_empty() || c == "." || c == ".."))
    {
        return Err(Error::InvalidPath(s.into()));
    }
    Ok(s.into())
}
#[cfg(target_os = "linux")]
pub(crate) fn sensitive(p: &Path) -> bool {
    let blocked = [
        ".codex",
        ".claude",
        ".ssh",
        ".config",
        ".agy",
        ".pi",
        ".opencode",
        ".ollama",
        ".gemini",
        ".agents",
        ".aws",
        ".kube",
        ".gnupg",
        ".netrc",
        ".npmrc",
        ".pypirc",
    ];
    p.components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        blocked.contains(&name.as_str())
            || name == ".env"
            || name.starts_with(".env.")
            || ["auth.json", "credential", "token", "keyring"]
                .iter()
                .any(|x| name.contains(x))
    })
}
#[cfg(target_os = "linux")]
mod inspect;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod queue;
#[cfg(target_os = "linux")]
pub use inspect::{duplicates, scan};
#[cfg(target_os = "linux")]
pub use queue::Queue;

#[cfg(not(target_os = "linux"))]
pub fn scan(_: &Path, _: &Limits, _: &Cancellation) -> Result<Scan> {
    Err(Error::UnsupportedPlatform)
}
#[cfg(not(target_os = "linux"))]
pub fn duplicates(_: &[Entry], _: &Limits, _: &Cancellation) -> Result<Vec<Vec<String>>> {
    Err(Error::UnsupportedPlatform)
}
#[cfg(not(target_os = "linux"))]
pub struct Queue;
#[cfg(not(target_os = "linux"))]
impl Queue {
    pub fn new(_: QueueOptions) -> Result<Self> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn preview(&mut self, _: &[String]) -> Result<Preview> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn commit(&mut self, _: &str, _: bool, _: &Cancellation) -> Result<Vec<History>> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn restore(&mut self, _: &str, _: &Cancellation) -> Result<Vec<History>> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn history(&self) -> &[History] {
        &[]
    }
    pub fn discard_preview(&mut self, _: &str) -> bool {
        false
    }
}
