//! The AppImage launcher points PATH, LD_LIBRARY_PATH, PYTHONHOME, GTK and
//! other variables into its bundle. The app's own GTK/WebKit need that, but
//! commands it runs (python3, du, git, cleanup tools) must see the user's
//! system as it is. Every child process gets these variables restored.
use std::{ffi::OsString, process::Command, sync::OnceLock};

/// Changes that undo the bundle: (name, None) removes, (name, Some) sets.
pub fn changes() -> &'static [(OsString, Option<OsString>)] {
    static CHANGES: OnceLock<Vec<(OsString, Option<OsString>)>> = OnceLock::new();
    CHANGES.get_or_init(|| {
        let Some(appdir) = std::env::var("APPDIR").ok().filter(|d| d.starts_with('/')) else {
            return Vec::new();
        };
        changes_for(&appdir, std::env::vars_os())
    })
}

fn changes_for(
    appdir: &str,
    vars: impl Iterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, Option<OsString>)> {
    let inside = |p: &str| p == appdir || p.starts_with(&format!("{appdir}/"));
    vars.filter_map(|(k, v)| {
        let v = v.to_str()?;
        let parts: Vec<&str> = v.split(':').filter(|p| !p.is_empty()).collect();
        if !parts.iter().any(|p| inside(p)) {
            return None;
        }
        let kept: Vec<&str> = parts.into_iter().filter(|p| !inside(p)).collect();
        Some((k, (!kept.is_empty()).then(|| kept.join(":").into())))
    })
    .collect()
}

pub trait HostEnv {
    /// Run with the user's environment, not the AppImage bundle's.
    fn host_env(&mut self) -> &mut Self;
}
impl HostEnv for Command {
    fn host_env(&mut self) -> &mut Self {
        for (k, v) in changes() {
            match v {
                Some(v) => self.env(k, v),
                None => self.env_remove(k),
            };
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bundle_entries_and_keeps_the_users() {
        let vars = [
            (
                "PATH",
                "/app/usr/bin/:/app/usr/sbin/:/usr/local/bin:/usr/bin",
            ),
            ("PYTHONHOME", "/app/usr/"),
            ("LD_LIBRARY_PATH", "/app/usr/lib/:/app//usr/lib64/:"),
            ("HOME", "/home/u"),
            (
                "XDG_DATA_DIRS",
                "/app/usr/share:/usr/share:/usr/local/share",
            ),
            ("OTHER", "/apple/x"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let mut got = changes_for("/app", vars.into_iter());
        got.sort();
        assert_eq!(
            got,
            vec![
                ("LD_LIBRARY_PATH".into(), None),
                ("PATH".into(), Some("/usr/local/bin:/usr/bin".into())),
                ("PYTHONHOME".into(), None),
                (
                    "XDG_DATA_DIRS".into(),
                    Some("/usr/share:/usr/local/share".into())
                ),
            ]
        );
    }
}
