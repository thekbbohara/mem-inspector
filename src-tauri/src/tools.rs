//! Cleanup commands the app itself generates, and the terminal that runs them.
//!
//! The UI never sends a command line. It sends a card id; the command comes
//! from this fixed list, built for the scanned home. Commands that need root
//! are shown, never run: nothing here calls sudo, doas, pkexec, su or run0.
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Tool {
    Pacman,
    Journal,
    DockerBuild,
    DockerImages,
    Uv,
    GoMod,
}

pub struct Command {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

const ROOT_TOOLS: &[&str] = &["sudo", "doas", "pkexec", "su", "run0"];

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Pacman,
        Tool::Journal,
        Tool::DockerBuild,
        Tool::DockerImages,
        Tool::Uv,
        Tool::GoMod,
    ];
    /// The pseudo path a card uses for this tool.
    pub fn key(self) -> &'static str {
        match self {
            Tool::Pacman => "system:pacman",
            Tool::Journal => "system:journal",
            Tool::DockerBuild => "system:docker-build",
            Tool::DockerImages => "system:docker-images",
            Tool::Uv => "system:uv",
            Tool::GoMod => "system:go-mod",
        }
    }
    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.key() == key)
    }
    pub fn what(self) -> &'static str {
        match self {
            Tool::Pacman => "pacman package cache (needs root)",
            Tool::Journal => "systemd journal logs (needs root)",
            Tool::DockerBuild => "Docker build cache",
            Tool::DockerImages => "Docker images no container uses",
            Tool::Uv => "uv package cache",
            Tool::GoMod => "Go module cache",
        }
    }
    pub fn needs_root(self) -> bool {
        matches!(self, Tool::Pacman | Tool::Journal)
    }
    /// Uses the data type colours: system-wide junk is "other".
    pub fn kind(self) -> &'static str {
        match self {
            Tool::Uv | Tool::GoMod => "caches",
            Tool::DockerBuild | Tool::DockerImages => "deps",
            _ => "other",
        }
    }
    fn home_path(self, home: &Path) -> Option<PathBuf> {
        match self {
            Tool::Uv => Some(home.join(".cache/uv")),
            Tool::GoMod => Some(home.join("go/pkg/mod")),
            _ => None,
        }
    }
    /// Touches only the scanned home, so it is safe in a rehearsal.
    pub fn home_scoped(self) -> bool {
        self.home_path(Path::new("/")).is_some()
    }
    /// The exact command shown on the card.
    pub fn line(self, home: &Path) -> String {
        match self {
            Tool::Pacman => {
                if on_path("paccache") {
                    "sudo paccache -rk1".into()
                } else {
                    "sudo pacman -Sc".into()
                }
            }
            Tool::Journal => "sudo journalctl --vacuum-size=200M".into(),
            _ => {
                let c = self.command(home).expect("runnable tool");
                let env: String = c
                    .env
                    .iter()
                    .map(|(k, v)| format!("{k}={} ", quote(v)))
                    .collect();
                let argv: Vec<String> = c.argv.iter().map(|a| quote(a)).collect();
                format!("{env}{}", argv.join(" "))
            }
        }
    }
    /// What actually runs. None for commands that need root.
    pub fn command(self, home: &Path) -> Option<Command> {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let home_str = |p: PathBuf| p.to_string_lossy().into_owned();
        let c = match self {
            Tool::Pacman | Tool::Journal => return None,
            Tool::DockerBuild => Command {
                argv: s(&["docker", "builder", "prune", "--force"]),
                env: vec![],
            },
            Tool::DockerImages => Command {
                argv: s(&["docker", "image", "prune", "--all", "--force"]),
                env: vec![],
            },
            Tool::Uv => Command {
                argv: vec![
                    "uv".into(),
                    "cache".into(),
                    "clean".into(),
                    "--cache-dir".into(),
                    home_str(home.join(".cache/uv")),
                ],
                env: vec![],
            },
            Tool::GoMod => Command {
                argv: s(&["go", "clean", "-modcache"]),
                env: vec![("GOMODCACHE".into(), home_str(home.join("go/pkg/mod")))],
            },
        };
        debug_assert!(!ROOT_TOOLS.contains(&c.argv[0].as_str()));
        Some(c)
    }
    fn available(self, home: &Path) -> bool {
        match self {
            Tool::Pacman => Path::new("/var/cache/pacman/pkg").is_dir(),
            Tool::Journal => Path::new("/var/log/journal").is_dir(),
            Tool::DockerBuild | Tool::DockerImages => {
                on_path("docker")
                    && std::process::Command::new("docker")
                        .args(["info", "--format", "{{.ID}}"])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .is_ok_and(|s| s.success())
            }
            Tool::Uv => on_path("uv") && self.home_path(home).is_some_and(|p| p.is_dir()),
            Tool::GoMod => on_path("go") && self.home_path(home).is_some_and(|p| p.is_dir()),
        }
    }
    /// Bytes the command would free right now.
    pub fn probe(self, home: &Path) -> u64 {
        match self {
            Tool::Pacman => du(Path::new("/var/cache/pacman/pkg")),
            Tool::Journal => du(Path::new("/var/log/journal")),
            Tool::DockerBuild => docker_reclaimable("Build Cache"),
            Tool::DockerImages => docker_reclaimable("Images"),
            Tool::Uv | Tool::GoMod => self.home_path(home).map(|p| du(&p)).unwrap_or(0),
        }
    }
}

/// Every available tool with what it would free. `real_home` is false in a
/// rehearsal, where only home-scoped tools are offered.
pub fn found(home: &Path, real_home: bool) -> Vec<(Tool, u64)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = Tool::ALL
            .into_iter()
            .filter(|t| real_home || t.home_scoped())
            .map(|t| s.spawn(move || (t, t.available(home).then(|| t.probe(home)))))
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter_map(|(t, b)| b.map(|b| (t, b)))
            .collect()
    })
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

fn quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c))
    {
        s.into()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn du(path: &Path) -> u64 {
    std::process::Command::new("du")
        .args(["-sx", "-B1", "--"])
        .arg(path)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split('\t')
                .next()?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// Docker reports SI units ("13.32GB (56%)").
pub fn parse_docker_size(text: &str) -> u64 {
    let t = text.split_whitespace().next().unwrap_or("");
    let split = t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().unwrap_or(0.0);
    let mult = match unit.to_ascii_uppercase().as_str() {
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => 1.0,
    };
    (n * mult) as u64
}

fn docker_reclaimable(kind: &str) -> u64 {
    std::process::Command::new("docker")
        .args(["system", "df", "--format", "{{.Type}}\t{{.Reclaimable}}"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .find(|(t, _)| *t == kind)
                .map(|(_, v)| parse_docker_size(v))
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// One command running in a pseudo terminal, its output kept for scrollback.
pub struct Run {
    pub id: String,
    pub card: String,
    pub line: String,
    pub output: Arc<Mutex<Vec<u8>>>,
    pub exit: Arc<Mutex<Option<i32>>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}

impl Run {
    pub fn start(
        id: String,
        card: String,
        line: String,
        cmd: Command,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> Result<Run, String> {
        use portable_pty::{CommandBuilder, PtySize};
        if cmd.argv.is_empty() || ROOT_TOOLS.contains(&cmd.argv[0].as_str()) {
            return Err("This command needs root; run it yourself".into());
        }
        let pty = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Terminal unavailable: {e}"))?;
        let mut builder = CommandBuilder::new(&cmd.argv[0]);
        builder.args(&cmd.argv[1..]);
        builder.cwd(cwd);
        builder.env("TERM", "xterm-256color");
        for (k, v) in &cmd.env {
            builder.env(k, v);
        }
        let mut child = pty
            .slave
            .spawn_command(builder)
            .map_err(|e| format!("{}: {e}", cmd.argv[0]))?;
        drop(pty.slave);
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let output = Arc::new(Mutex::new(
            format!("\x1b[2m$ {line}\x1b[0m\r\n").into_bytes(),
        ));
        let exit = Arc::new(Mutex::new(None));
        let (out, ex) = (output.clone(), exit.clone());
        std::thread::spawn(move || {
            let master = pty.master;
            let mut buf = [0u8; 8192];
            let reading = std::thread::spawn({
                let out = out.clone();
                move || {
                    while let Ok(n) = reader.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        if let Ok(mut o) = out.lock() {
                            o.extend_from_slice(&buf[..n]);
                        }
                    }
                }
            });
            let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
            drop(master);
            let _ = reading.join();
            if let Ok(mut e) = ex.lock() {
                *e = Some(code);
            }
        });
        Ok(Run {
            id,
            card,
            line,
            output,
            exit,
            killer: Mutex::new(killer),
        })
    }
    pub fn stop(&self) {
        if let Ok(mut k) = self.killer.lock() {
            let _ = k.kill();
        }
    }
    pub fn done(&self) -> Option<i32> {
        self.exit.lock().ok().and_then(|e| *e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_sizes_are_si() {
        assert_eq!(parse_docker_size("13.32GB (56%)"), 13_320_000_000);
        assert_eq!(parse_docker_size("4.551MB (99%)"), 4_551_000);
        assert_eq!(parse_docker_size("0B"), 0);
    }

    #[test]
    fn root_commands_are_shown_never_run() {
        let home = Path::new("/home/x");
        for t in Tool::ALL {
            match t.command(home) {
                Some(c) => {
                    assert!(!t.needs_root());
                    assert!(!ROOT_TOOLS.contains(&c.argv[0].as_str()));
                    assert!(!t.line(home).contains("sudo"));
                }
                None => {
                    assert!(t.needs_root());
                    assert!(t.line(home).starts_with("sudo "));
                }
            }
            assert_eq!(Tool::from_key(t.key()), Some(t));
        }
        assert_eq!(
            Tool::Uv.line(home),
            "uv cache clean --cache-dir /home/x/.cache/uv"
        );
        assert_eq!(
            Tool::GoMod.line(home),
            "GOMODCACHE=/home/x/go/pkg/mod go clean -modcache"
        );
        let sudo = Command {
            argv: vec!["sudo".into(), "true".into()],
            env: vec![],
        };
        assert!(Run::start(
            "r".into(),
            "c".into(),
            "sudo true".into(),
            sudo,
            home,
            80,
            24
        )
        .is_err());
    }

    #[test]
    fn runs_in_a_pty_and_keeps_output_and_exit_code() {
        let cmd = Command {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf 'hello\\n'; [ -t 1 ] && echo tty; exit 3".into(),
            ],
            env: vec![],
        };
        let run = Run::start(
            "r1".into(),
            "c".into(),
            "demo".into(),
            cmd,
            Path::new("/"),
            80,
            24,
        )
        .unwrap();
        let start = std::time::Instant::now();
        while run.done().is_none() {
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(run.done(), Some(3));
        let text = String::from_utf8_lossy(&run.output.lock().unwrap()).into_owned();
        assert!(
            text.contains("$ demo") && text.contains("hello") && text.contains("tty"),
            "{text:?}"
        );
    }
}
