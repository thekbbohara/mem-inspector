//! Native CLI adapters. No shell interpolation, credential copying or account fallback.
use super::{Advice, Error, Provider, LIMIT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const ENGINES: &[&str] = &["claude", "codex", "kimi", "agy", "opencode", "pi"];
const PI_FLAGS: &[&str] = &[
    "--offline",
    "--no-tools",
    "--no-mcp",
    "--no-extensions",
    "--no-skills",
    "--no-context-files",
    "--no-prompt-templates",
    "--no-themes",
    "--no-approve",
    "--no-session",
];

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineSettings {
    #[serde(default)]
    pub model: String,
    /// Existing CLI configuration folder. Never a credential, account ID or shell command.
    #[serde(default)]
    pub account: String,
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}
/// The CLI's own config folder: its env override when set and absolute,
/// else the folder the CLI uses by default.
fn default_account(name: &str) -> PathBuf {
    let env = match name {
        "claude" => "CLAUDE_CONFIG_DIR",
        "codex" => "CODEX_HOME",
        _ => "",
    };
    if let Some(dir) = (!env.is_empty())
        .then(|| std::env::var_os(env))
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return dir;
    }
    home().join(match name {
        "claude" => ".claude",
        "codex" => ".codex",
        "kimi" => ".kimi-code",
        "pi" => ".pi/agent",
        _ => "",
    })
}
fn account(name: &str, settings: Option<&EngineSettings>) -> Result<PathBuf, Error> {
    let input = settings.map(|s| s.account.as_str()).unwrap_or("");
    let path = if input.is_empty() {
        default_account(name)
    } else if let Some(s) = input.strip_prefix("~/") {
        home().join(s)
    } else {
        PathBuf::from(input)
    };
    if !path.is_absolute() || input.len() > 4096 || input.chars().any(char::is_control) {
        return Err(blocker(
            "Invalid account folder; use an existing absolute path or ~/ path",
        ));
    }
    if matches!(name, "agy" | "opencode") && !input.is_empty() {
        return Err(blocker(
            "This CLI uses its native account store; alternate account folders are unsupported",
        ));
    }
    Ok(path)
}
fn blocker(reason: &str) -> Error {
    Error::Blocker(reason.into())
}

/// PATH without the AppImage bundle's folders, so the user's own CLI runs.
fn host_path() -> Option<std::ffi::OsString> {
    let path = std::env::var_os("PATH")?;
    let Some(appdir) = std::env::var("APPDIR").ok().filter(|d| d.starts_with('/')) else {
        return Some(path);
    };
    let kept: Vec<PathBuf> = std::env::split_paths(&path)
        .filter(|p| !p.starts_with(&appdir))
        .collect();
    std::env::join_paths(kept).ok()
}

pub(super) fn base_command(
    name: &str,
    settings: Option<&EngineSettings>,
) -> Result<Command, Error> {
    let mut c = Command::new(name);
    c.env_clear();
    if let Some(path) = host_path() {
        c.env("PATH", path);
    }
    for key in ["HOME", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
        if let Some(v) = std::env::var_os(key) {
            c.env(key, v);
        }
    }
    let dir = account(name, settings)?;
    match name {
        "claude" => {
            // Only pass a folder that was chosen: Claude's built-in default
            // also reads ~/.claude.json, which CLAUDE_CONFIG_DIR=~/.claude would skip.
            let chosen = settings.is_some_and(|s| !s.account.is_empty())
                || std::env::var_os("CLAUDE_CONFIG_DIR").is_some();
            if chosen {
                c.env("CLAUDE_CONFIG_DIR", dir);
            }
            c.env("CLAUDE_CODE_SAFE_MODE", "1");
        }
        "codex" => {
            c.env("CODEX_HOME", dir);
        }
        "kimi" => {
            c.env("KIMI_CODE_HOME", dir);
        }
        "pi" => {
            c.env("PI_CODING_AGENT_DIR", dir).env("PI_TELEMETRY", "0");
        }
        "agy" => {
            c.env("AGY_CLI_HIDE_ACCOUNT_INFO", "1");
        }
        "opencode" => {
            c.env("OPENCODE_DISABLE_AUTOUPDATE", "true")
                .env("OPENCODE_DISABLE_SHARE", "true");
        }
        _ => return Err(Error::Unavailable),
    }
    Ok(c)
}
struct Output {
    stdout: String,
    stderr: String,
    success: bool,
}
/// Bounded readers and a process group prevent a CLI/helper from outliving timeout.
fn execute(
    mut c: Command,
    cwd: &Path,
    input: Option<&str>,
    timeout: Duration,
) -> Result<Output, Error> {
    c.current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = c.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::CliMissing
        } else {
            Error::InvocationFailed
        }
    })?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    for (kind, mut stream) in [
        (
            false,
            Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        ),
        (
            true,
            Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
        ),
    ] {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stream.by_ref().take(LIMIT + 1).read_to_end(&mut bytes);
            let result = if result.is_err() {
                Err(Error::InvocationFailed)
            } else if bytes.len() > LIMIT as usize {
                Err(Error::InvalidResponse)
            } else {
                String::from_utf8(bytes).map_err(|_| Error::InvalidResponse)
            };
            let _ = tx.send((kind, result));
        });
    }
    drop(tx);
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(input) = input {
            if stdin.write_all(input.as_bytes()).is_err() {
                kill_group(pid);
                let _ = child.wait();
                return Err(Error::InvocationFailed);
            }
        }
    }
    let start = Instant::now();
    let mut output = HashMap::new();
    loop {
        while let Ok((kind, result)) = rx.try_recv() {
            match result {
                Ok(text) => {
                    output.insert(kind, text);
                }
                Err(e) => {
                    kill_group(pid);
                    let _ = child.wait();
                    return Err(e);
                }
            }
        }
        if start.elapsed() >= timeout {
            kill_group(pid);
            let _ = child.wait();
            return Err(Error::TimedOut);
        }
        if let Some(status) = child.try_wait().map_err(|_| Error::InvocationFailed)? {
            while output.len() < 2 {
                let (kind, result) = rx
                    .recv_timeout(timeout.saturating_sub(start.elapsed()))
                    .map_err(|_| {
                        kill_group(pid);
                        Error::TimedOut
                    })?;
                output.insert(kind, result?);
            }
            kill_group(pid);
            return Ok(Output {
                stdout: output.remove(&false).unwrap(),
                stderr: output.remove(&true).unwrap(),
                success: status.success(),
            });
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn kill_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
fn probe_output(
    name: &str,
    settings: Option<&EngineSettings>,
    args: &[&str],
) -> Result<String, Error> {
    let dir = tempfile::tempdir().map_err(|_| Error::InvocationFailed)?;
    let mut c = base_command(name, settings)?;
    c.args(args);
    let out = execute(c, dir.path(), None, Duration::from_secs(12))?;
    if out.success {
        Ok(if args.contains(&"--help") || args == ["login", "status"] {
            strip_ansi(&format!("{}\n{}", out.stdout, out.stderr))
        } else {
            out.stdout
        })
    } else {
        Err(classify(&out.stdout, &out.stderr))
    }
}
/// Only fixed categories leave the process boundary, never raw stderr or account metadata.
fn classify(out: &str, err: &str) -> Error {
    let text = format!("{out}\n{err}").to_ascii_lowercase();
    let reason = if text.contains("429")
        || text.contains("usage limit")
        || text.contains("quota")
        || text.contains("rate limit")
        || text.contains("exhausted")
    {
        "Subscription/free quota or rate limit reached; wait for reset. No paid fallback attempted"
    } else if text.contains("401")
        || text.contains("unauthorized")
        || text.contains("not logged")
        || text.contains("no credentials")
        || text.contains("no credential configured")
        || text.contains("authentication")
        || text.contains("login required")
    {
        "Existing CLI authentication is unavailable or expired; no login or credential changes attempted"
    } else if text.contains("403")
        || text.contains("permission_denied")
        || text.contains("permission denied")
    {
        "Provider denied this account or model (403/permission denied); no account fallback attempted"
    } else if text.contains("model")
        && [
            "unsupported",
            "not supported",
            "not found",
            "not available",
            "unavailable",
            "not recognized",
        ]
        .iter()
        .any(|s| text.contains(s))
    {
        "Selected model is unavailable for this account; no model fallback attempted"
    } else if text.contains("503") || text.contains("overloaded") {
        "Provider service unavailable (503/overloaded); retry later"
    } else if text.contains("connect")
        || text.contains("network")
        || text.contains("dns")
        || text.contains("fetch failed")
    {
        "CLI could not reach the provider; check network availability"
    } else if text.contains("unknown option")
        || text.contains("unknown configuration")
        || text.contains("invalid value")
        || text.contains("invalid type")
        || text.contains("unknown feature")
    {
        "Installed CLI rejected the adapter configuration; unsupported CLI contract"
    } else {
        "CLI failed without a recognized safe error category; raw output withheld"
    };
    blocker(reason)
}
fn read_json(path: &Path) -> Result<Value, Error> {
    let text = fs::read_to_string(path)
        .map_err(|_| blocker("Local model catalogue unavailable; no model identifier guessed"))?;
    serde_json::from_str(&text)
        .map_err(|_| blocker("Local model catalogue malformed; no model identifier guessed"))
}
fn kimi_config(dir: &Path) -> Result<toml::Value, Error> {
    fs::read_to_string(dir.join("config.toml"))
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            blocker("Kimi configuration unavailable or malformed; no model identifier guessed")
        })
}
fn valid_model(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 150
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/_.:-".contains(&c))
}
fn models(name: &str, settings: Option<&EngineSettings>) -> Result<Vec<String>, Error> {
    let dir = account(name, settings)?;
    let rows: Vec<String> = match name {
        "claude" => {
            let help = probe_output(name, settings, &["--help"])?;
            ["fable", "opus", "sonnet"]
                .iter()
                .filter(|m| help.contains(&format!("'{m}'")))
                .map(|s| (*s).into())
                .collect()
        }
        "codex" => read_json(&dir.join("models_cache.json"))?["models"]
            .as_array()
            .ok_or(Error::InvalidResponse)?
            .iter()
            .filter(|m| m["visibility"] == "list")
            .filter_map(|m| m["slug"].as_str().map(String::from))
            .collect(),
        "kimi" => kimi_config(&dir)?["models"]
            .as_table()
            .ok_or(Error::InvalidResponse)?
            .keys()
            .cloned()
            .collect(),
        "agy" => probe_output(name, settings, &["models"])?
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(id, _)| id.into()))
            .collect(),
        "pi" => {
            let mut flags = PI_FLAGS.to_vec();
            flags.push("--list-models");
            probe_output(name, settings, &flags)?
                .lines()
                .skip(1)
                .filter_map(|l| {
                    let mut c = l.split_whitespace();
                    let provider = c.next()?;
                    let model = c.next()?;
                    // API-key/credit providers are intentionally outside this subscription adapter.
                    matches!(provider, "openai-codex" | "anthropic")
                        .then(|| format!("{provider}/{model}"))
                })
                .collect()
        }
        "opencode" => free_opencode_models(settings)?,
        _ => return Err(Error::Unavailable),
    };
    Ok(rows.into_iter().filter(|m| valid_model(m)).collect())
}
fn free_opencode_models(settings: Option<&EngineSettings>) -> Result<Vec<String>, Error> {
    let text = probe_output(
        "opencode",
        settings,
        &["models", "opencode", "--verbose", "--pure"],
    )?;
    let mut rows = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("opencode/") {
            continue;
        }
        let mut object = String::new();
        while lines.peek().is_some_and(|l| !l.starts_with("opencode/")) {
            object.push_str(lines.next().unwrap());
            object.push('\n');
        }
        let m: Value = serde_json::from_str(&object).map_err(|_| Error::InvalidResponse)?;
        if m["status"] == "active"
            && ["input", "output"]
                .iter()
                .all(|k| m["cost"][k].as_f64() == Some(0.0))
            && ["read", "write"]
                .iter()
                .all(|k| m["cost"]["cache"][k].as_f64() == Some(0.0))
        {
            rows.push(line.into());
        }
    }
    Ok(rows)
}
fn readiness(
    name: &str,
    version: &str,
    settings: &EngineSettings,
    listed: &[String],
) -> Result<(), Error> {
    let help_args = if matches!(name, "codex") {
        vec!["exec", "--help"]
    } else if name == "opencode" {
        vec!["run", "--help"]
    } else {
        vec!["--help"]
    };
    let help = probe_output(name, Some(settings), &help_args)?;
    let flags: &[&str] = match name {
        "claude" => super::FLAGS,
        "codex" => &[
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--json",
            "--sandbox",
        ],
        "kimi" => &[
            "--agent-file",
            "--skills-dir",
            "--output-format",
            "--prompt",
        ],
        "agy" => &[
            "--agent",
            "--disable-slash-commands",
            "--output-format",
            "--print-timeout",
        ],
        "opencode" => &["--pure", "--agent", "--format", "--model"],
        "pi" => PI_FLAGS,
        _ => return Err(Error::Unavailable),
    };
    let missing: Vec<_> = flags
        .iter()
        .filter(|f| !super::has_flag(&help, f))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(blocker(&format!(
            "Installed CLI lacks required isolation flags: {}; unsupported CLI contract",
            missing.join(", ")
        )));
    }
    if name != "claude" && listed.is_empty() {
        return Err(blocker(
            "No eligible subscription/free models discovered; no model identifier guessed",
        ));
    }
    if !settings.model.is_empty() && !listed.contains(&settings.model) {
        return Err(blocker("Selected model is not in this CLI's eligible catalogue; refresh discovery or select a listed model"));
    }
    match name {
        "claude" => {
            if super::compatible_version(version).is_none() {
                return Err(blocker("Unsupported Claude CLI version; requires the reviewed 2.1 patch contract (2.1.296 or newer)"));
            }
            if !super::supported(&help) {
                return Err(blocker("Claude help does not confirm the reviewed tools, customization and MCP isolation semantics"));
            }
            let auth: Value = serde_json::from_str(&probe_output(
                name,
                Some(settings),
                &["auth", "status", "--json"],
            )?)
            .map_err(|_| Error::InvalidResponse)?;
            if auth["loggedIn"] != true {
                return Err(blocker(
                    "Claude subscription login is not verified; no credential changes made",
                ));
            }
            if auth["authMethod"] != "claude.ai" || auth["apiProvider"] != "firstParty" {
                return Err(blocker("Claude authentication is not a verified first-party subscription; API billing is unsupported"));
            }
        }
        "codex" => {
            if version.trim() != "codex-cli 0.162.1" {
                return Err(blocker(
                    "Unsupported Codex CLI contract; reviewed version is 0.162.1",
                ));
            }
            let auth = probe_output(name, Some(settings), &["login", "status"])?;
            if !auth.contains("Logged in using ChatGPT") {
                return Err(blocker(
                    "Codex ChatGPT subscription login is not verified; API billing is unsupported",
                ));
            }
        }
        "kimi" => {
            if version.trim() != "2.1.1" {
                return Err(blocker(
                    "Unsupported Kimi CLI contract; reviewed version is 2.1.1",
                ));
            }
            let config = kimi_config(&account(name, Some(settings))?)?;
            let alias = selected_model(name, settings, listed);
            let provider = config["models"][&alias]["provider"]
                .as_str()
                .ok_or(Error::InvalidResponse)?;
            let p = &config["providers"][provider];
            if p.get("api_key")
                .and_then(toml::Value::as_str)
                .is_some_and(|k| !k.is_empty())
                || p.get("oauth").is_none()
            {
                return Err(blocker("Selected Kimi provider is not configured for existing OAuth; API billing is unsupported"));
            }
            // No token reads or refresh. Authentication is ultimately verified by the CLI turn.
        }
        "pi" => {
            if version.trim() != "1.0.4" {
                return Err(blocker(
                    "Unsupported pi CLI contract; reviewed version is 1.0.4",
                ));
            }
            let model = selected_model(name, settings, listed);
            let provider = model.split('/').next().ok_or(Error::InvalidResponse)?;
            let a: Value = serde_json::from_str(&probe_output(
                name,
                Some(settings),
                &[
                    "auth",
                    "check",
                    "--provider",
                    provider,
                    "--no-refresh",
                    "--json",
                ],
            )?)
            .map_err(|_| Error::InvalidResponse)?;
            if a["status"] != "ready" || a["authType"] != "oauth" {
                return Err(blocker("Selected pi provider has no ready OAuth subscription; API billing is unsupported"));
            }
        }
        "agy" if version.trim() != "1.3.5" => {
            return Err(blocker(
                "Unsupported agy CLI contract; reviewed version is 1.3.5",
            ))
        }
        "opencode" if version.trim() != "1.18.35" => {
            return Err(blocker(
                "Unsupported OpenCode CLI contract; reviewed version is 1.18.35",
            ))
        }
        _ => {}
    }
    Ok(())
}
fn selected_model(name: &str, settings: &EngineSettings, listed: &[String]) -> String {
    if !settings.model.is_empty() {
        return settings.model.clone();
    }
    if name == "pi" {
        return listed
            .iter()
            .find(|m| m.as_str() == "openai-codex/gpt-6-luna")
            .or_else(|| listed.iter().find(|m| m.starts_with("openai-codex/")))
            .cloned()
            .unwrap_or_default();
    }
    if name == "kimi" {
        if let Ok(c) = kimi_config(&account(name, Some(settings)).unwrap_or_default()) {
            if let Some(m) = c["default_model"]
                .as_str()
                .filter(|m| listed.iter().any(|v| v == m))
            {
                return m.into();
            }
        }
    }
    if name == "claude" {
        return String::new();
    }
    listed.first().cloned().unwrap_or_default()
}
fn discover_one(name: &str, settings: Option<&EngineSettings>) -> Provider {
    let mut settings = settings.cloned().unwrap_or_default();
    let mut p = Provider {
        name: name.into(),
        installed: false,
        usable: false,
        reason: String::new(),
        status: "missing".into(),
        models: vec![],
        settings: settings.clone(),
    };
    let result = (|| {
        let version = probe_output(name, None, &["--version"])?;
        if version.trim().is_empty() {
            return Err(Error::InvocationFailed);
        }
        p.installed = true;
        p.status = "unsupported".into();
        p.models = models(name, Some(&settings))?;
        readiness(name, &version, &settings, &p.models)?;
        settings.model = selected_model(name, &settings, &p.models);
        p.settings = settings;
        Ok(())
    })();
    match result {
        Ok(()) => {
            p.usable = true;
            p.status = if matches!(name, "kimi" | "agy" | "opencode") {
                "auth-unverified"
            } else {
                "ready"
            }
            .into();
            p.reason = match name { "claude" => "Claude 2.1: compatible isolation flags and subscription login; runtime checked on each request", "codex" | "pi" => "Existing OAuth subscription verified; quota checked only by inference", "opencode" => "CLI lists zero-cost models, but the provider may deny them (403) at request time; not verified without a live request; no paid model or fallback permitted", _ => "Tool-free adapter ready; existing OAuth/account and quota verified only by inference" }.into();
        }
        Err(e) => {
            p.reason = e.to_string();
            if matches!(e, Error::TimedOut) {
                p.status = "timeout".into();
            } else if p.reason.to_ascii_lowercase().contains("auth") || p.reason.contains("login") {
                p.status = "auth-unavailable".into();
            }
        }
    }
    p
}
pub fn discover_with(provider: Option<&str>, settings: Option<&EngineSettings>) -> Vec<Provider> {
    ENGINES
        .iter()
        .map(|name| {
            discover_one(
                name,
                if provider == Some(*name) {
                    settings
                } else {
                    None
                },
            )
        })
        .collect()
}

pub(super) fn infer(
    name: &str,
    settings: Option<&EngineSettings>,
    system: &str,
    input: &str,
) -> Result<Advice, Error> {
    if !ENGINES.contains(&name) {
        return Err(Error::Unavailable);
    }
    let p = discover_one(name, settings);
    if !p.usable {
        return Err(blocker(&format!("{name}: {}", p.reason)));
    }
    let settings = &p.settings;
    let dir = tempfile::tempdir().map_err(|_| Error::InvocationFailed)?;
    let mut c = base_command(name, Some(settings))?;
    let prompt = format!("{system}\n\nInput data:\n{input}");
    match name {
        "claude" => {
            let mut args: Vec<String> = super::arguments().iter().map(|s| (*s).into()).collect();
            let index = args.iter().position(|s| s == "--system-prompt").unwrap() + 1;
            args[index] = system.into();
            if !settings.model.is_empty() {
                args.extend(["--model".into(), settings.model.clone()]);
            }
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = super::run_checked_with(
                name,
                &refs,
                Some(input),
                Duration::from_secs(90),
                true,
                Some(settings),
            )?;
            return super::parse_advice(name, &out);
        }
        "codex" => {
            codex_command(&mut c, dir.path(), settings, system)?;
        }
        "pi" => {
            c.args(PI_FLAGS)
                .args([
                    "-p",
                    "--mode",
                    "json",
                    "--model",
                    &settings.model,
                    "--system-prompt",
                    system,
                ])
                .arg(&prompt);
        }
        "kimi" => {
            prepare_kimi(&mut c, dir.path(), settings, system)?;
            c.args([
                "--prompt",
                &prompt,
                "--output-format",
                "stream-json",
                "--model",
                &settings.model,
            ]);
        }
        "agy" => {
            let folder = dir.path().join(".agents/agents/mem-inspector");
            fs::create_dir_all(&folder).map_err(|_| Error::InvocationFailed)?;
            let spec = json!({"name":"mem-inspector","description":"Anonymous storage proposal","mainAgent":true,"subagent":false,"inheritCustomizations":false,"inheritMcp":false,"excludeDefaultComponents":true,"tools":[],"mcpServers":[],"commandExecutionPolicy":"off"});
            fs::write(
                folder.join("agent.md"),
                format!("---\n{spec}\n---\n# System Prompt\n{system}\n"),
            )
            .map_err(|_| Error::InvocationFailed)?;
            c.args([
                "--agent",
                "mem-inspector",
                "--model",
                &settings.model,
                "--output-format",
                "json",
                "--disable-slash-commands",
                "--print-timeout",
                "80s",
            ])
            .arg(format!("--print={prompt}"));
        }
        "opencode" => {
            let config = json!({"$schema":"https://opencode.ai/config.json","autoupdate":false,"share":"disabled","plugin":[],"mcp":{},"instructions":[],"permission":{"*":"deny"},"agent":{"mem-inspector":{"mode":"primary","prompt":system,"tools":{"*":false},"permission":{"*":"deny"}}}});
            c.env("OPENCODE_CONFIG_CONTENT", config.to_string())
                .env("OPENCODE_DISABLE_GLOBAL_CONFIG", "true")
                .env("OPENCODE_DISABLE_PROJECT_CONFIG", "true")
                .env("OPENCODE_DISABLE_CLAUDE_CODE", "true")
                .env("OPENCODE_DISABLE_SKILLS", "true");
            c.args([
                "run",
                "--pure",
                "--format",
                "json",
                "--agent",
                "mem-inspector",
                "--model",
                &settings.model,
                "--title",
                "Mem Inspector anonymous fixture",
            ])
            .arg(&prompt);
        }
        _ => return Err(Error::Unavailable),
    }
    let out = execute(
        c,
        dir.path(),
        (name == "codex").then_some(prompt.as_str()),
        Duration::from_secs(90),
    )?;
    if !out.success {
        return Err(classify(&out.stdout, &out.stderr));
    }
    let text = parse_output(name, &out.stdout)?;
    checked_text(&text)?;
    Ok(Advice {
        provider: name.into(),
        text,
    })
}
fn codex_command(
    c: &mut Command,
    dir: &Path,
    settings: &EngineSettings,
    system: &str,
) -> Result<(), Error> {
    let mut catalogue = read_json(&account("codex", Some(settings))?.join("models_cache.json"))?;
    let models = catalogue["models"]
        .as_array_mut()
        .ok_or(Error::InvalidResponse)?;
    // Preserve discovered identifiers and routing; strip model-specific tool sources.
    for m in models {
        m["apply_patch_tool_type"] = Value::Null;
        m["experimental_supported_tools"] = json!([]);
        m["tool_mode"] = Value::Null;
        m["include_skills_usage_instructions"] = json!(false);
        m["include_plugin_usage_instructions"] = json!(false);
        m["include_apps_usage_instructions"] = json!(false);
    }
    let path = dir.join("models.json");
    fs::write(&path, json!({"models":catalogue["models"]}).to_string())
        .map_err(|_| Error::InvocationFailed)?;
    c.args([
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--sandbox",
        "read-only",
        "--model",
        &settings.model,
    ]);
    for flag in [
        "shell_tool",
        "unified_exec",
        "view_image",
        "apps",
        "hooks",
        "remote_plugin",
        "recommended_plugins",
        "skill_search",
        "skill_mcp_dependency_install",
        "browser_use",
        "computer_use",
        "image_generation",
        "multi_agent",
        "multi_agent_v2",
        "goals",
        "sleep_tool",
        "code_mode",
        "code_mode_host",
        "code_mode_only",
        "request_permissions_tool",
        "js_repl",
        "in_app_browser",
        "in_app_chat",
    ] {
        c.args(["--disable", flag]);
    }
    for setting in [
        "web_search=\"disabled\"".into(),
        "tools.update_plan.enabled=false".into(),
        "tools.experimental_request_user_input.enabled=false".into(),
        "project_doc_max_bytes=0".into(),
        "mcp_servers={}".into(),
        "plugins={}".into(),
        "skills.config=[]".into(),
        "model_reasoning_effort=\"low\"".into(),
        "model_provider=\"openai\"".into(),
        "forced_login_method=\"chatgpt\"".into(),
        format!("model_catalog_json={}", json!(path)),
        format!("developer_instructions={}", json!(system)),
    ] {
        c.args(["-c", &setting]);
    }
    c.arg("-");
    Ok(())
}
fn prepare_kimi(
    c: &mut Command,
    dir: &Path,
    settings: &EngineSettings,
    system: &str,
) -> Result<(), Error> {
    let source = account("kimi", Some(settings))?;
    let config = kimi_config(&source)?;
    let provider = config["models"][&settings.model]["provider"]
        .as_str()
        .ok_or(Error::InvalidResponse)?;
    let mut safe = toml::map::Map::new();
    safe.insert(
        "default_model".into(),
        toml::Value::String(settings.model.clone()),
    );
    safe.insert("models".into(), config["models"].clone());
    let mut providers = toml::map::Map::new();
    providers.insert(provider.into(), config["providers"][provider].clone());
    safe.insert("providers".into(), toml::Value::Table(providers));
    let isolated = dir.join("kimi-home");
    fs::create_dir(&isolated).map_err(|_| Error::InvocationFailed)?;
    fs::write(
        isolated.join("config.toml"),
        toml::to_string(&safe).map_err(|_| Error::InvalidResponse)?,
    )
    .map_err(|_| Error::InvocationFailed)?;
    // Native CLI reads its existing OAuth store; no credentials are read, copied or changed here.
    #[cfg(unix)]
    for name in ["oauth", "credentials", "region"] {
        let source = source.join(name);
        if source.exists() {
            std::os::unix::fs::symlink(source, isolated.join(name))
                .map_err(|_| Error::InvocationFailed)?;
        }
    }
    fs::write(isolated.join("mcp.json"), "{\"mcpServers\":{}}")
        .map_err(|_| Error::InvocationFailed)?;
    let profile = dir.join("agent.md");
    fs::write(&profile, format!("---\nname: mem-inspector\ndescription: Anonymous numeric storage proposal\ntools: []\nsubagents: []\n---\n{system}\n")).map_err(|_| Error::InvocationFailed)?;
    let skills = dir.join("empty-skills");
    fs::create_dir(&skills).map_err(|_| Error::InvocationFailed)?;
    c.env("KIMI_CODE_HOME", isolated)
        .args(["--agent-file"])
        .arg(profile)
        .arg("--skills-dir")
        .arg(skills);
    Ok(())
}
fn checked_text(text: &str) -> Result<(), Error> {
    if text.trim().is_empty()
        || text.len() > 16384
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err(Error::InvalidResponse)
    } else {
        Ok(())
    }
}
fn parse_output(name: &str, output: &str) -> Result<String, Error> {
    if name == "agy" {
        let v: Value = serde_json::from_str(output).map_err(|_| Error::InvalidResponse)?;
        if v["status"] != "SUCCESS" {
            return Err(classify(output, ""));
        }
        return v["response"]
            .as_str()
            .map(String::from)
            .ok_or(Error::InvalidResponse);
    }
    let mut text = String::new();
    let mut completed = false;
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|_| Error::InvalidResponse)?;
        match name {
            "codex" => match v["type"].as_str() {
                Some("thread.started" | "turn.started") if !completed => {}
                Some("item.started" | "item.updated" | "item.completed") if !completed => {
                    if !matches!(
                        v["item"]["type"].as_str(),
                        Some("agent_message" | "reasoning")
                    ) {
                        return Err(Error::InvalidResponse);
                    }
                    if v["type"] == "item.completed" && v["item"]["type"] == "agent_message" {
                        text = v["item"]["text"]
                            .as_str()
                            .ok_or(Error::InvalidResponse)?
                            .into();
                    }
                }
                Some("turn.completed") if !completed => completed = true,
                Some("error" | "turn.failed") => return Err(classify(line, "")),
                _ => return Err(Error::InvalidResponse),
            },
            "pi" => match v["type"].as_str() {
                Some("message_end") => {
                    let m = &v["message"];
                    if m["role"] == "assistant" {
                        if m["stopReason"] == "error" || m["stopReason"] == "aborted" {
                            return Err(classify(line, ""));
                        }
                        text = text_blocks(&m["content"])?;
                    }
                }
                Some("agent_end") => completed = true,
                Some("tool_execution_start" | "tool_execution_end" | "tool_execution_update") => {
                    return Err(Error::InvalidResponse)
                }
                Some(
                    "session"
                    | "agent_start"
                    | "turn_start"
                    | "turn_end"
                    | "message_start"
                    | "message_update"
                    | "model_change"
                    | "thinking_level_change"
                    | "agent_settled",
                ) => {
                    if contains_tool(&v) {
                        return Err(Error::InvalidResponse);
                    }
                }
                _ => return Err(Error::InvalidResponse),
            },
            "kimi" => {
                if v["role"] == "tool" || v.get("tool_calls").is_some() {
                    return Err(Error::InvalidResponse);
                }
                if v["role"] == "assistant" {
                    text.push_str(v["content"].as_str().ok_or(Error::InvalidResponse)?);
                    completed = true;
                } else if v["role"] == "meta"
                    && matches!(
                        v["type"].as_str(),
                        Some("system.version" | "session.resume_hint")
                    )
                {
                } else {
                    return Err(classify(line, ""));
                }
            }
            "opencode" => match v["type"].as_str() {
                Some("text") => {
                    text.push_str(v["part"]["text"].as_str().ok_or(Error::InvalidResponse)?)
                }
                Some("step_finish") => {
                    if v["part"]["reason"] != "stop" {
                        return Err(Error::InvalidResponse);
                    }
                    completed = true;
                }
                Some("step_start" | "reasoning") => {}
                Some("error") => return Err(classify(line, "")),
                _ => return Err(Error::InvalidResponse),
            },
            _ => return Err(Error::InvalidResponse),
        }
    }
    if !completed {
        return Err(Error::InvalidResponse);
    }
    checked_text(&text)?;
    Ok(text)
}
fn contains_tool(v: &Value) -> bool {
    match v {
        Value::Object(m) => {
            m.get("type")
                .and_then(Value::as_str)
                .is_some_and(|s| matches!(s, "toolCall" | "tool_use" | "toolResult"))
                || m.values().any(contains_tool)
        }
        Value::Array(a) => a.iter().any(contains_tool),
        _ => false,
    }
}
fn text_blocks(blocks: &Value) -> Result<String, Error> {
    let mut text = String::new();
    for b in blocks.as_array().ok_or(Error::InvalidResponse)? {
        match b["type"].as_str() {
            Some("text") => text.push_str(b["text"].as_str().ok_or(Error::InvalidResponse)?),
            Some("thinking") => {}
            _ => return Err(Error::InvalidResponse),
        }
    }
    Ok(text)
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_protocols_accept_text_and_reject_execution_events() {
        let examples = [
            ("codex", "{\"type\":\"thread.started\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"ok\"}}\n{\"type\":\"turn.completed\"}"),
            ("pi", "{\"type\":\"agent_start\"}\n{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}],\"stopReason\":\"stop\"}}\n{\"type\":\"agent_end\"}"),
            ("kimi", "{\"role\":\"meta\",\"type\":\"system.version\"}\n{\"role\":\"assistant\",\"content\":\"ok\"}\n{\"role\":\"meta\",\"type\":\"session.resume_hint\"}"),
            ("opencode", "{\"type\":\"step_start\"}\n{\"type\":\"text\",\"part\":{\"text\":\"ok\"}}\n{\"type\":\"step_finish\",\"part\":{\"reason\":\"stop\"}}"),
            ("agy", "{\"status\":\"SUCCESS\",\"response\":\"ok\"}"),
        ];
        for (name, output) in examples {
            assert_eq!(parse_output(name, output).unwrap(), "ok", "{name}");
        }
        for (name, output) in [
            ("codex", "{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"command\":\"private\"}}"),
            ("pi", "{\"type\":\"message_update\",\"event\":{\"type\":\"toolCall\"}}"),
            ("pi", "{\"type\":\"tool_execution_start\"}"),
            ("kimi", "{\"role\":\"assistant\",\"content\":\"ok\",\"tool_calls\":[]}"),
            ("kimi", "{\"role\":\"tool\",\"content\":\"private\"}"),
            ("opencode", "{\"type\":\"tool_use\"}"),
            ("opencode", "{\"type\":\"step_finish\",\"part\":{\"reason\":\"tool-calls\"}}"),
        ] { assert!(parse_output(name, output).is_err(), "{name}"); }
        for name in ["codex", "pi", "kimi", "opencode", "agy"] {
            assert!(parse_output(name, "bad json").is_err());
            assert!(parse_output(name, "").is_err());
        }
    }
    #[test]
    fn safe_blockers_never_echo_identifiers_or_credentials() {
        for (reason, word) in [
            (
                "401 auth-token-secret account@fixture.invalid",
                "authentication",
            ),
            ("429 exhausted account-secret", "quota"),
            ("403 permission denied account-secret", "denied"),
            ("503 overloaded secret", "503"),
            ("selected model unavailable private-model", "model"),
            ("unknown option secret", "contract"),
            (
                "no credential configured private-provider",
                "authentication",
            ),
        ] {
            let error = classify(reason, "secret-stderr").to_string();
            assert!(error.contains(word));
            for secret in ["secret", "fixture.invalid", "private-", "auth-token"] {
                assert!(!error.contains(secret));
            }
        }
    }
    #[test]
    fn invalid_settings_fail_before_model_or_process_execution() {
        assert!(account(
            "codex",
            Some(&EngineSettings {
                account: "relative".into(),
                ..Default::default()
            })
        )
        .is_err());
        assert!(account(
            "codex",
            Some(&EngineSettings {
                account: "/tmp/\nprivate".into(),
                ..Default::default()
            })
        )
        .is_err());
        assert!(account(
            "agy",
            Some(&EngineSettings {
                account: "/tmp/existing".into(),
                ..Default::default()
            })
        )
        .is_err());
        for model in ["-m foo", "x;cmd", "$(private)", "\n", "@private", ""] {
            assert!(!valid_model(model));
        }
        assert!(valid_model("openai-codex/gpt-6.1-sol"));
        assert!(serde_json::from_value::<EngineSettings>(
            json!({"model":"ok", "api_key":"fixture-secret"})
        )
        .is_err());
    }
    #[test]
    fn readiness_text_is_not_runtime_quota_evidence() {
        assert!(classify("usage limit", "")
            .to_string()
            .contains("No paid fallback"));
        assert!(parse_output(
            "codex",
            "{\"type\":\"error\",\"message\":\"429 fixture-secret\"}"
        )
        .unwrap_err()
        .to_string()
        .contains("quota"));
        assert!(parse_output("pi", "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"error\",\"errorMessage\":\"403 fixture-secret\"}}").unwrap_err().to_string().contains("denied"));
    }
    #[test]
    fn process_deadline_and_output_caps_include_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Command::new("/bin/sleep");
        c.arg("2");
        let start = Instant::now();
        assert!(matches!(
            execute(c, dir.path(), None, Duration::from_millis(40)),
            Err(Error::TimedOut)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut c = Command::new("/usr/bin/python3");
        c.args(["-c", "import sys;sys.stderr.write('x'*70000)"]);
        assert!(matches!(
            execute(c, dir.path(), None, Duration::from_secs(2)),
            Err(Error::InvalidResponse)
        ));
    }
    #[test]
    fn kimi_profile_excludes_hooks_services_and_customizations() {
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("credentials")).unwrap();
        let config = r#"default_model = "fixture/model"
[hooks]
malicious = "fixture-command"
[services]
malicious = "fixture-url"
[providers."managed:fixture"]
type = "kimi"
api_key = ""
[providers."managed:fixture".oauth]
storage = "file"
key = "oauth/fixture"
[models."fixture/model"]
provider = "managed:fixture"
model = "fixture"
max_context_size = 32000
"#;
        fs::write(source.path().join("config.toml"), config).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let settings = EngineSettings {
            model: "fixture/model".into(),
            account: source.path().to_string_lossy().into(),
        };
        let mut c = base_command("kimi", Some(&settings)).unwrap();
        prepare_kimi(&mut c, dir.path(), &settings, "numeric-only").unwrap();
        let generated = fs::read_to_string(dir.path().join("kimi-home/config.toml")).unwrap();
        assert!(
            !generated.contains("malicious")
                && !generated.contains("hooks")
                && !generated.contains("services")
        );
        assert!(fs::read_to_string(dir.path().join("agent.md"))
            .unwrap()
            .contains("tools: []"));
        assert_eq!(
            fs::read_to_string(source.path().join("config.toml")).unwrap(),
            config
        );
        assert_eq!(
            fs::read_link(dir.path().join("kimi-home/credentials")).unwrap(),
            source.path().join("credentials")
        );
        assert!(dir
            .path()
            .join("empty-skills")
            .read_dir()
            .unwrap()
            .next()
            .is_none());
    }
}
