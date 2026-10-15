//! Placement advisor: data on the fast SSD that belongs on the slow, roomy
//! HDD. Everything here is a candidate for `Space::archive` (verified copy,
//! symlink back, undo). Evidence comes from the scan; each candidate is then
//! rechecked against the move guard, so dotfiles, config, tracked git files,
//! files in use and other owners never show up.
use crate::space::{self, Kind, OpenFiles, Space};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const DAY: u64 = 86400;
const MIN_MEDIA: u64 = 100 << 20;
const MIN_ARCHIVE: u64 = 100 << 20;
const MIN_MODEL: u64 = 500 << 20;
const MIN_PROJECT: u64 = 200 << 20;
const MIN_ANDROID: u64 = 300 << 20;
const MIN_DOWNLOAD: u64 = 50 << 20;
const MIN_IDLE_FILE: u64 = 1 << 30;
pub const DOWNLOAD_DAYS: u64 = 30;
const MODEL_IDLE_DAYS: u64 = 14;
const IDLE_FILE_DAYS: u64 = 90;

const VIDEO: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "flv", "mts", "m2ts", "mpg", "mpeg",
    "prores", "mxf", "braw", "r3d", "wav", "flac", "aiff", "mp3", "m4a", "blend", "exr", "psd",
];
const ARCHIVE: &[&str] = &[
    "zip", "tar", "gz", "tgz", "xz", "txz", "zst", "bz2", "7z", "rar", "iso", "img", "dmg",
    "qcow2", "vdi", "vmdk", "vhd", "vhdx", "ova", "sql", "dump", "bak",
];
const MODEL: &[&str] = &[
    "gguf",
    "safetensors",
    "ckpt",
    "pt",
    "pth",
    "onnx",
    "ggml",
    "h5",
    "tflite",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Projects,
    Android,
    Models,
    Media,
    Archives,
    Downloads,
    Idle,
}

impl Group {
    pub const ALL: [Group; 7] = [
        Group::Media,
        Group::Projects,
        Group::Models,
        Group::Archives,
        Group::Downloads,
        Group::Android,
        Group::Idle,
    ];
    /// Data type, for colour and the disk bar.
    pub fn kind(self) -> &'static str {
        match self {
            Group::Projects => "code",
            Group::Android => "deps",
            Group::Models => "models",
            Group::Media => "video",
            Group::Archives => "archives",
            Group::Downloads | Group::Idle => "other",
        }
    }
    pub fn what(self) -> &'static str {
        match self {
            Group::Projects => "idle git project",
            Group::Android => "Android system image",
            Group::Models => "AI model not loaded lately",
            Group::Media => "large video or media file",
            Group::Archives => "archive, backup or disk image",
            Group::Downloads => "old download",
            Group::Idle => "big file untouched 90+ days",
        }
    }
    pub fn title(self, n: usize) -> String {
        let s = |one: &str, many: &str| {
            if n == 1 {
                one.to_string()
            } else {
                format!("{n} {many}")
            }
        };
        match self {
            Group::Projects => s("Idle project", "idle projects"),
            Group::Android => s("Android system image", "Android system images"),
            Group::Models => s("AI model you rarely load", "AI models you rarely load"),
            Group::Media => s("Large video", "large videos and media"),
            Group::Archives => s("Archive or backup", "archives, backups and images"),
            Group::Downloads => format!("Downloads older than {DOWNLOAD_DAYS} days"),
            Group::Idle => s(
                "Big file untouched 90+ days",
                "big files untouched 90+ days",
            ),
        }
    }
    /// Card title; a lone project or download also says its name.
    pub fn title_for(self, paths: &[String]) -> String {
        let base = self.title(paths.len());
        match (self, paths) {
            (Group::Projects | Group::Downloads, [one]) => {
                let name = Path::new(one)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                match (self, name) {
                    (Group::Projects, Some(n)) => format!("{base} {n}"),
                    (Group::Downloads, Some(n)) => format!("Old download {n}"),
                    _ => base,
                }
            }
            _ => base,
        }
    }
    pub fn tradeoff(self) -> &'static str {
        match self {
            Group::Projects => "Builds run slower there; move it back when you pick it up",
            Group::Android => "Emulator boots slower; fine for images you rarely run",
            Group::Models => "Loads slower; fine for models you rarely run",
            Group::Media => "Opens a bit slower; fine for footage you rarely open",
            Group::Archives => "Extracting is slower; nothing else changes",
            Group::Downloads => "Opens a bit slower; you have not touched them in a month",
            Group::Idle => "Opens slower the rare time you need it",
        }
    }
}

/// What stays on the SSD, shown once under the placement cards.
pub const KEEP_ON_SSD: &str = "Keep on the SSD: the OS, active projects, node_modules, caches, browser profiles, Docker and models you use daily. They need fast random reads.";

#[derive(Debug, Clone, Serialize)]
pub struct Place {
    pub path: String,
    pub bytes: u64,
    pub group: Group,
    pub dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_days: Option<u64>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn num(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.as_u64())
        .unwrap_or(0)
}

fn ext(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn days_since(t: i64) -> u64 {
    now().saturating_sub(t.max(0) as u64) / DAY
}

/// Candidates by group, biggest first, each one passing the move guard.
pub fn find(space: &Space, overview: &Value) -> Vec<Place> {
    let open = OpenFiles::scan();
    let mut raw: Vec<Place> = Vec::new();
    let home = &space.home;

    for r in overview["repos"].as_array().into_iter().flatten() {
        let Some(p) = r["path"].as_str() else {
            continue;
        };
        let bytes = num(&r["allocated"]);
        let newest = r["newest"].as_i64().unwrap_or(0);
        if bytes < MIN_PROJECT || days_since(newest) < space::PROJECT_IDLE_DAYS {
            continue;
        }
        let days = space::project_idle_days(Path::new(p));
        if days >= space::PROJECT_IDLE_DAYS {
            raw.push(Place {
                path: p.into(),
                bytes,
                group: Group::Projects,
                dir: true,
                idle_days: Some(days.min(days_since(newest))),
            });
        }
    }

    let images = home.join("Android/Sdk/system-images");
    for api in read_dirs(&images) {
        for tag in read_dirs(&api) {
            for abi in read_dirs(&tag) {
                let bytes = space::freeable(std::slice::from_ref(&abi));
                if bytes >= MIN_ANDROID {
                    raw.push(Place {
                        path: abi.to_string_lossy().into(),
                        bytes,
                        group: Group::Android,
                        dir: true,
                        idle_days: None,
                    });
                }
            }
        }
    }

    for f in overview["bigFiles"].as_array().into_iter().flatten() {
        let Some(p) = f["path"].as_str() else {
            continue;
        };
        let bytes = num(&f["allocated"]);
        let (mtime, atime) = (
            f["mtime"].as_i64().unwrap_or(0),
            f["atime"].as_i64().unwrap_or(0),
        );
        let e = ext(p);
        let in_model_store = p.contains("/.ollama/models/blobs/");
        let group = if (MODEL.contains(&e.as_str()) || in_model_store) && bytes >= MIN_MODEL {
            (days_since(atime) >= MODEL_IDLE_DAYS).then_some(Group::Models)
        } else if VIDEO.contains(&e.as_str()) && bytes >= MIN_MEDIA {
            Some(Group::Media)
        } else if ARCHIVE.contains(&e.as_str()) && bytes >= MIN_ARCHIVE {
            Some(Group::Archives)
        } else if bytes >= MIN_IDLE_FILE
            && days_since(mtime) >= IDLE_FILE_DAYS
            && days_since(atime) >= IDLE_FILE_DAYS
        {
            Some(Group::Idle)
        } else {
            None
        };
        if let Some(group) = group {
            raw.push(Place {
                path: p.into(),
                bytes,
                group,
                dir: false,
                idle_days: Some(days_since(mtime.max(atime))),
            });
        }
    }

    for d in overview["downloads"].as_array().into_iter().flatten() {
        let Some(p) = d["path"].as_str() else {
            continue;
        };
        let bytes = num(&d["allocated"]);
        let newest = d["newest"].as_i64().unwrap_or(i64::MAX);
        if bytes >= MIN_DOWNLOAD && days_since(newest) >= DOWNLOAD_DAYS {
            raw.push(Place {
                path: p.into(),
                bytes,
                group: Group::Downloads,
                dir: d["dir"].as_bool().unwrap_or(false),
                idle_days: Some(days_since(newest)),
            });
        }
    }

    // Folders first so files inside them are not offered twice.
    raw.sort_by_key(|p| (!p.dir, std::cmp::Reverse(p.bytes)));
    let mut taken: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for p in raw {
        let path = PathBuf::from(&p.path);
        if taken
            .iter()
            .any(|t| path.starts_with(t) || t.starts_with(&path))
        {
            continue;
        }
        if space.guard_with(Kind::Move, &path, &open).is_err() {
            continue;
        }
        if space
            .archived(&path)
            .is_some_and(|a| fs::symlink_metadata(a).is_ok())
        {
            continue;
        }
        taken.push(path);
        out.push(p);
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.bytes));
    out
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

/// Bytes still on the SSD for a placement path, or None when it is gone or
/// already a link (moved).
pub fn live_bytes(path: &Path) -> Option<u64> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    Some(if meta.is_dir() {
        space::freeable(&[path.to_path_buf()])
    } else {
        meta.blocks() * 512
    })
}

/// The shared HDD folder for a set of paths, for "SSD -> HDD (...)".
pub fn target(archive: &Path, paths: &[String]) -> String {
    let mut common: Option<PathBuf> = None;
    for p in paths {
        let parent = Path::new(p)
            .parent()
            .unwrap_or(Path::new("/"))
            .to_path_buf();
        common = Some(match common {
            None => parent,
            Some(c) => {
                let mut c = c;
                while !parent.starts_with(&c) {
                    if !c.pop() {
                        break;
                    }
                }
                c
            }
        });
    }
    let base = common.unwrap_or_else(|| PathBuf::from("/"));
    archive
        .join(base.strip_prefix("/").unwrap_or(&base))
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(path: &Path, n: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![3u8; n]).unwrap();
    }
    fn age(path: &Path, days: u64) {
        let t = libc::timespec {
            tv_sec: (now() - days * DAY) as i64,
            tv_nsec: 0,
        };
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        unsafe {
            libc::utimensat(
                libc::AT_FDCWD,
                c.as_ptr(),
                [t, t].as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
    }
    fn big(path: &Path, bytes: u64, days: u64) -> Value {
        json!({"path": path, "allocated": bytes.to_string(), "mtime": now() as i64 - (days * DAY) as i64, "atime": now() as i64 - (days * DAY) as i64})
    }

    #[test]
    fn finds_media_models_archives_downloads_projects_and_skips_unsafe() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = Space::new(home.clone(), Some(dir.path().join("archive"))).unwrap();
        let video = home.join("Videos/trip.mkv");
        let model = home.join("models/llama.gguf");
        let fresh_model = home.join("models/daily.gguf");
        let iso = home.join("isos/arch.iso");
        let dotted = home.join(".local/share/big.mkv");
        let config = home.join(".config/app/clip.mp4");
        let tracked = home.join("repo/demo.mp4");
        let ignored = home.join("repo/out/render.mp4");
        for p in [
            &video,
            &model,
            &fresh_model,
            &iso,
            &dotted,
            &config,
            &tracked,
            &ignored,
        ] {
            write(p, 10);
        }
        write(&home.join("repo/.gitignore"), 0);
        fs::write(home.join("repo/.gitignore"), "out/\n").unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(home.join("repo"))
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["add", "demo.mp4", ".gitignore"]);
        let project = home.join("code/old-site");
        write(&project.join("index.html"), 10);
        fs::create_dir_all(project.join(".git")).unwrap();
        write(&project.join(".git/HEAD"), 10);
        for p in [
            project.join("index.html"),
            project.join(".git/HEAD"),
            project.join(".git"),
            project.clone(),
        ] {
            age(&p, 100);
        }
        let old_dl = home.join("Downloads/installer");
        write(&old_dl.join("setup.run"), 10);
        age(&old_dl.join("setup.run"), 60);
        age(&old_dl, 60);
        let gb = 1u64 << 30;
        let overview = json!({
            "bigFiles": [
                big(&video, gb, 1), big(&model, gb, 40), big(&fresh_model, gb, 1),
                big(&iso, gb, 5), big(&dotted, gb, 300), big(&config, gb, 300),
                big(&tracked, gb, 300), big(&ignored, gb, 300),
            ],
            "repos": [{"path": project, "allocated": (300u64 << 20).to_string(), "newest": now() as i64 - 100 * DAY as i64}],
            "downloads": [{"path": old_dl, "allocated": (60u64 << 20).to_string(), "newest": now() as i64 - 60 * DAY as i64, "dir": true}],
        });
        let found = find(&space, &overview);
        let by = |g: Group| -> Vec<String> {
            found
                .iter()
                .filter(|p| p.group == g)
                .map(|p| {
                    p.path
                        .trim_start_matches(home.to_str().unwrap())
                        .to_string()
                })
                .collect()
        };
        assert_eq!(
            by(Group::Media),
            vec!["/Videos/trip.mkv", "/repo/out/render.mp4"],
            "{found:?}"
        );
        assert_eq!(by(Group::Models), vec!["/models/llama.gguf"]);
        assert_eq!(by(Group::Archives), vec!["/isos/arch.iso"]);
        assert_eq!(by(Group::Projects), vec!["/code/old-site"]);
        assert_eq!(by(Group::Downloads), vec!["/Downloads/installer"]);
        let all: Vec<_> = found
            .iter()
            .map(|p| {
                p.path
                    .trim_start_matches(home.to_str().unwrap())
                    .to_string()
            })
            .collect();
        assert!(
            !all.iter()
                .any(|p| p.contains("/.") || p.ends_with("demo.mp4")),
            "{all:?}"
        );
        assert_eq!(
            target(
                space.archive.as_deref().unwrap(),
                &[video.to_string_lossy().into()]
            ),
            space
                .archived(&home.join("Videos"))
                .unwrap()
                .to_string_lossy()
        );
    }

    #[test]
    fn files_open_in_a_running_process_are_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = Space::new(home.clone(), Some(dir.path().join("archive"))).unwrap();
        let video = home.join("Videos/live.mp4");
        write(&video, 10);
        let open = OpenFiles::with(vec![video.clone()]);
        assert!(space
            .guard_with(Kind::Move, &video, &open)
            .unwrap_err()
            .contains("running app"));
        assert!(space
            .guard_with(Kind::Move, &home.join("Videos"), &open)
            .is_err());
        // A real open handle in another process is seen through /proc.
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("exec 3<\"$0\"; sleep 5")
            .arg(&video)
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(OpenFiles::scan().touches(&video));
        child.kill().unwrap();
        child.wait().unwrap();
    }
}

/// Read-only: what the advisor would offer on a real scan.
/// `MEM_OVERVIEW_JSON=scan.json cargo test real_home_placement -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore = "reads the real home"]
fn real_home_placement() {
    let path = std::env::var("MEM_OVERVIEW_JSON").expect("MEM_OVERVIEW_JSON");
    let overview: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let space = Space::from_env().unwrap();
    let t = std::time::Instant::now();
    let found = find(&space, &overview);
    println!("placement {:?}, {} items", t.elapsed(), found.len());
    for p in found {
        println!(
            "PLACE {:?} {} {} -> {}",
            p.group,
            crate::advisor::human(p.bytes),
            p.path,
            space
                .archive
                .as_deref()
                .map(|a| target(a, std::slice::from_ref(&p.path)))
                .unwrap_or_default()
        );
    }
}
