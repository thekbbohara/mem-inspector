//! The user's choices, kept in `$XDG_CONFIG_HOME/mem-inspector/settings.json`
//! (default `~/.config/mem-inspector/settings.json`).
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Folder on a second disk where personal files are moved. None: moves off.
    pub archive: Option<PathBuf>,
    /// The first-run question was answered (a folder or "cleanup only").
    pub setup_done: bool,
    /// Extra folders, relative to home, that are never touched or listed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub protect: Vec<PathBuf>,
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("mem-inspector"))
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }
    pub fn load() -> Self {
        Self::path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }
    fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path().ok_or("Home directory unavailable")?)
    }
    /// Write a sibling temp file, then rename, so a crash never leaves half a file.
    fn save_to(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("Invalid settings path")?;
        fs::create_dir_all(dir).map_err(|e| format!("Cannot save settings: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::File::create(&tmp)
            .and_then(|mut f| {
                f.write_all(text.as_bytes())?;
                f.sync_all()
            })
            .and_then(|_| fs::rename(&tmp, path))
            .map_err(|e| format!("Cannot save settings: {e}"))
    }
}

/// The user's extra protected folders, read once per run.
pub fn protected_extra() -> &'static [PathBuf] {
    static EXTRA: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    EXTRA.get_or_init(|| {
        Settings::load()
            .protect
            .into_iter()
            .filter(|p| p.is_relative() && !p.as_os_str().is_empty())
            .collect()
    })
}

/// Where Claude Code keeps its login: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude")
        })
}

/// True when an executable with this name is on PATH.
pub fn on_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            fs::metadata(dir.join(name))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_file_means_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mem-inspector/settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
    }

    #[test]
    fn saves_and_loads_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mem-inspector/settings.json");
        let s = Settings {
            archive: Some("/media/disk/Archive".into()),
            setup_done: true,
            protect: vec![".agents".into()],
        };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        assert!(!path.with_extension("json.tmp").exists());
        let off = Settings {
            archive: None,
            setup_done: true,
            ..Default::default()
        };
        off.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), off);
    }
}
