use crate::{
    linux::{self, Identity},
    path_text, sensitive, Cancellation, Entry, Error, History, Preview, QueueOptions, Result,
};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_ITEMS: usize = 500;
const MAX_REVIEWS: usize = 32;
const MAX_HISTORY: usize = 10_000;
struct Reviewed {
    entry: Entry,
    identity: Identity,
    parent: Identity,
}
struct Review {
    items: Vec<Reviewed>,
}
struct Record {
    dto: History,
    identity: Identity,
    parent: Identity,
    name: String,
}
pub struct Queue {
    options: QueueOptions,
    root: File,
    root_identity: Identity,
    trash: File,
    files: File,
    info: File,
    protected: Vec<PathBuf>,
    reviews: HashMap<String, Review>,
    records: Vec<Record>,
    history: Vec<History>,
}
fn intersects(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn private(dir: &File) -> Result<()> {
    if !linux::owned_private(dir)? {
        return Err(Error::Protected(
            "Trash directories must be owned by current user and mode 0700".into(),
        ));
    }
    Ok(())
}
fn git_guard(p: &Path) -> Result<()> {
    let mut cur = Some(p);
    while let Some(p) = cur {
        let dir = linux::open_dir(p)?;
        if linux::exists_at(&dir, ".git")? {
            return Err(Error::Protected("Active git worktree".into()));
        }
        cur = p.parent();
    }
    Ok(())
}
impl Queue {
    pub fn new(options: QueueOptions) -> Result<Self> {
        path_text(&options.root)?;
        path_text(&options.trash)?;
        if options.root == Path::new("/") || intersects(&options.root, &options.trash) {
            return Err(Error::Protected(
                "Root/Trash overlap or filesystem root".into(),
            ));
        }
        if options.fixture
            && !options
                .root
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("mem-core-fixture-"))
        {
            return Err(Error::Protected(
                "Fixture root must be a disposable mem-core-fixture-* directory".into(),
            ));
        }
        let mut protected: Vec<PathBuf> = [
            "/etc", "/usr", "/var", "/boot", "/proc", "/sys", "/dev", "/run", "/opt",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            if options.root == home {
                return Err(Error::Protected("Home root".into()));
            }
            for part in [".local/share", ".local/state"] {
                protected.push(home.join(part));
            }
        }
        if !options.fixture {
            protected.push(std::env::current_dir()?);
            git_guard(&options.root)?;
        }
        protected.extend(options.protected_paths.iter().cloned());
        for p in &protected {
            path_text(p)?;
        }
        if sensitive(&options.root) || protected.iter().any(|p| intersects(p, &options.root)) {
            return Err(Error::Protected(options.root.display().to_string()));
        }
        let root = linux::open_dir(&options.root)?;
        if !linux::owned_no_external_write(&root)? {
            return Err(Error::Protected(
                "Cleanup root must be user-owned and not writable by group/others".into(),
            ));
        }
        let root_identity = Identity::of(&root.metadata()?);
        let trash = linux::open_dir(&options.trash)?;
        private(&trash)?;
        if root_identity.dev != trash.metadata()?.dev() {
            return Err(Error::Protected("Cross-device Trash unsupported".into()));
        }
        linux::mkdir_at(&trash, "files")?;
        linux::mkdir_at(&trash, "info")?;
        let files = linux::open_at(&trash, "files", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        let info = linux::open_at(&trash, "info", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        private(&files)?;
        private(&info)?;
        if files.metadata()?.dev() != root_identity.dev
            || info.metadata()?.dev() != root_identity.dev
        {
            return Err(Error::Protected("Trash mount boundary".into()));
        }
        Ok(Self {
            options,
            root,
            root_identity,
            trash,
            files,
            info,
            protected,
            reviews: HashMap::new(),
            records: vec![],
            history: vec![],
        })
    }
    fn anchors(&self) -> Result<()> {
        if !self.root_identity.same_object(&Identity::of(
            &linux::open_dir(&self.options.root)?.metadata()?,
        )) {
            return Err(Error::Changed("Cleanup root replaced".into()));
        }
        let path = linux::open_dir(&self.options.trash)?;
        if !Identity::of(&self.trash.metadata()?).same_object(&Identity::of(&path.metadata()?)) {
            return Err(Error::Changed("Trash root replaced".into()));
        }
        private(&self.trash)?;
        private(&self.files)?;
        private(&self.info)?;
        for (name, fd) in [("files", &self.files), ("info", &self.info)] {
            let cur = linux::stat_at(&self.trash, name)?;
            if !Identity::of(&cur).same_object(&Identity::of(&fd.metadata()?)) {
                return Err(Error::Changed(format!("Trash {name} replaced")));
            }
        }
        Ok(())
    }
    fn allowed(&self, p: &Path) -> Result<()> {
        path_text(p)?;
        if p == self.options.root
            || !p.starts_with(&self.options.root)
            || sensitive(p)
            || self.protected.iter().any(|x| intersects(x, p))
            || intersects(&self.options.trash, p)
        {
            return Err(Error::Protected(p.display().to_string()));
        }
        if !self.options.fixture {
            git_guard(
                p.parent()
                    .ok_or_else(|| Error::InvalidPath("no parent".into()))?,
            )?;
        }
        Ok(())
    }
    fn source_parent(&self, p: &Path) -> Result<(File, String)> {
        self.allowed(p)?;
        let rel = p
            .strip_prefix(&self.options.root)
            .map_err(|_| Error::Protected(p.display().to_string()))?;
        let name = rel
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::InvalidPath("no name".into()))?
            .to_owned();
        let mut dir = linux::open_at(&self.root, ".", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        if !linux::owned_no_external_write(&dir)? {
            return Err(Error::Protected("Unsafe cleanup parent permissions".into()));
        }
        for part in rel.parent().into_iter().flat_map(|p| p.components()) {
            let name = part
                .as_os_str()
                .to_str()
                .ok_or_else(|| Error::InvalidPath("non-UTF-8".into()))?;
            dir = linux::open_at(&dir, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
            if !linux::owned_no_external_write(&dir)? {
                return Err(Error::Protected("Unsafe cleanup parent permissions".into()));
            }
            if dir.metadata()?.dev() != self.root_identity.dev {
                return Err(Error::Protected("Mount boundary".into()));
            }
        }
        Ok((dir, name))
    }
    fn inspect(&self, path: &str) -> Result<Reviewed> {
        let (parent, name) = self.source_parent(Path::new(path))?;
        let meta = linux::stat_at(&parent, &name)?;
        if !meta.is_file()
            || !linux::user_owned(&meta)
            || meta.nlink() != 1
            || meta.dev() != self.root_identity.dev
        {
            return Err(Error::Protected(
                "Only regular, non-hardlinked files on root device can enter cleanup".into(),
            ));
        }
        Ok(Reviewed {
            entry: linux::entry(path.into(), &meta, false),
            identity: Identity::of(&meta),
            parent: Identity::of(&parent.metadata()?),
        })
    }
    fn revalidate(&self, item: &Reviewed) -> Result<(File, String)> {
        let (parent, name) = self.source_parent(Path::new(&item.entry.path))?;
        if !item.parent.same_object(&Identity::of(&parent.metadata()?))
            || item.identity != Identity::of(&linux::stat_at(&parent, &name)?)
        {
            return Err(Error::Changed(item.entry.path.clone()));
        }
        Ok((parent, name))
    }
    pub fn preview(&mut self, paths: &[String]) -> Result<Preview> {
        self.anchors()?;
        if paths.is_empty()
            || paths.len() > MAX_ITEMS
            || paths.iter().collect::<HashSet<_>>().len() != paths.len()
        {
            return Err(Error::Limit("review requires 1..500 distinct paths".into()));
        }
        if self.reviews.len() >= MAX_REVIEWS {
            return Err(Error::Limit("live reviews".into()));
        }
        let items = paths
            .iter()
            .map(|p| self.inspect(p))
            .collect::<Result<Vec<_>>>()?;
        let id = uuid::Uuid::new_v4().to_string();
        let preview = Preview {
            id: id.clone(),
            items: items.iter().map(|r| r.entry.clone()).collect(),
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| Error::Io(e.to_string()))?
                .as_millis() as u64,
        };
        self.reviews.insert(id, Review { items });
        Ok(preview)
    }
    pub fn discard_preview(&mut self, id: &str) -> bool {
        self.reviews.remove(id).is_some()
    }
    pub fn history(&self) -> &[History] {
        &self.history
    }
    pub fn commit(
        &mut self,
        id: &str,
        confirmed: bool,
        cancel: &Cancellation,
    ) -> Result<Vec<History>> {
        if !confirmed {
            return Err(Error::ConfirmationRequired);
        }
        cancel.check()?;
        self.anchors()?;
        let review = self.reviews.get(id).ok_or(Error::UnknownReview)?;
        if self.history.len() + review.items.len() > MAX_HISTORY {
            return Err(Error::Limit("history records".into()));
        }
        for item in &review.items {
            cancel.check()?;
            self.revalidate(item)?;
        }
        // Consume before first mutation: partially completed batches cannot be replayed.
        let review = self.reviews.remove(id).ok_or(Error::UnknownReview)?;
        for item in review.items {
            cancel.check()?;
            self.anchors()?;
            let (parent, name) = self.revalidate(&item)?;
            let key = uuid::Uuid::new_v4().to_string();
            let info_name = format!("{key}.trashinfo");
            let info_path = self.options.trash.join("info").join(&info_name);
            let trash_path = self.options.trash.join("files").join(&key);
            let mut info = linux::open_at(
                &self.info,
                &info_name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            let encoded = percent_path(&item.entry.path);
            write!(
                info,
                "[Trash Info]\nPath={encoded}\nDeletionDate={}\n",
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
            )?;
            info.sync_all()?;
            // No further cancellation between move and validation/history recording.
            // This is a short non-interruptible recovery boundary, not a batch transaction.
            self.revalidate(&item)?;
            linux::rename(&parent, &name, &self.files, &key)?;
            let moved = linux::stat_at(&self.files, &key).map(|m| Identity::of(&m));
            if !moved.as_ref().is_ok_and(|m| item.identity.after_move(m)) {
                if linux::rename(&self.files, &key, &parent, &name).is_err() {
                    return Err(Error::Recovery {
                        original: item.entry.path,
                        retained: trash_path.display().to_string(),
                    });
                }
                return Err(Error::Changed(item.entry.path));
            }
            let identity = moved?;
            let dto = History {
                id: key.clone(),
                original: item.entry.path,
                trashed: trash_path.display().to_string(),
                info: info_path.display().to_string(),
                dev: identity.dev,
                ino: identity.ino,
                restored: false,
            };
            self.records.push(Record {
                dto: dto.clone(),
                identity,
                parent: item.parent,
                name: key,
            });
            self.history.push(dto);
            // Persist directory entries after history is recorded. If sync fails,
            // caller can still discover the completed move using history().
            self.files.sync_all()?;
            parent.sync_all()?;
            self.info.sync_all()?;
        }
        cancel.check()?;
        Ok(self.history.clone())
    }
    pub fn restore(&mut self, id: &str, cancel: &Cancellation) -> Result<Vec<History>> {
        cancel.check()?;
        self.anchors()?;
        let index = self
            .records
            .iter()
            .position(|r| r.dto.id == id && !r.dto.restored)
            .ok_or(Error::UnknownHistory)?;
        let record = &self.records[index];
        let (parent, name) = self.source_parent(Path::new(&record.dto.original))?;
        if !record
            .parent
            .same_object(&Identity::of(&parent.metadata()?))
            || record.identity != Identity::of(&linux::stat_at(&self.files, &record.name)?)
        {
            return Err(Error::Changed(record.dto.trashed.clone()));
        }
        linux::rename(&self.files, &record.name, &parent, &name)?;
        let restored = linux::stat_at(&parent, &name).map(|m| Identity::of(&m));
        if !restored
            .as_ref()
            .is_ok_and(|m| record.identity.after_move(m))
        {
            if linux::rename(&parent, &name, &self.files, &record.name).is_err() {
                return Err(Error::Recovery {
                    original: record.dto.trashed.clone(),
                    retained: record.dto.original.clone(),
                });
            }
            return Err(Error::Changed(record.dto.trashed.clone()));
        }
        self.records[index].dto.restored = true;
        self.history[index].restored = true;
        parent.sync_all()?;
        self.files.sync_all()?;
        cancel.check()?;
        Ok(self.history.clone())
    }
}
fn percent_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    use std::fmt::Write;
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            write!(&mut out, "%{b:02X}").expect("String writes cannot fail");
        }
    }
    out
}

#[cfg(test)]
mod safety_review_tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{symlink, PermissionsExt},
    };
    use tempfile::{Builder, TempDir};
    struct Fixture {
        root: TempDir,
        trash: TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/generated");
            fs::create_dir_all(&base).unwrap();
            let make = |prefix| {
                Builder::new()
                    .prefix(prefix)
                    .permissions(fs::Permissions::from_mode(0o700))
                    .tempdir_in(&base)
                    .unwrap()
            };
            Self {
                root: make("mem-core-fixture-"),
                trash: make("mem-core-trash-fixture-"),
            }
        }
        fn queue(&self) -> Queue {
            Queue::new(QueueOptions {
                root: self.root.path().into(),
                trash: self.trash.path().into(),
                protected_paths: vec![],
                fixture: true,
            })
            .unwrap()
        }
        fn file(&self, name: &str, data: &[u8]) -> String {
            let p = self.root.path().join(name);
            fs::write(&p, data).unwrap();
            p.to_str().unwrap().into()
        }
    }
    #[test]
    fn leaf_replacement_at_commit_syscall_is_rolled_back() {
        let f = Fixture::new();
        let p = f.file("reviewed", b"reviewed");
        let saved = f.root.path().join("saved-reviewed");
        let mut q = f.queue();
        let r = q.preview(std::slice::from_ref(&p)).unwrap();
        let source = p.clone();
        let saved_copy = saved.clone();
        linux::RENAME_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&source, saved_copy).unwrap();
                fs::write(&source, b"unreviewed").unwrap();
            }))
        });
        assert!(matches!(
            q.commit(&r.id, true, &Cancellation::default()),
            Err(Error::Changed(_))
        ));
        assert_eq!(fs::read(&p).unwrap(), b"unreviewed");
        assert_eq!(fs::read(saved).unwrap(), b"reviewed");
        assert!(q.history().is_empty());
        assert_eq!(
            fs::read_dir(f.trash.path().join("files")).unwrap().count(),
            0
        );
    }
    #[test]
    fn failed_rollback_retains_both_objects_and_reports_recovery() {
        let f = Fixture::new();
        let p = f.file("reviewed", b"reviewed");
        let saved = f.root.path().join("saved-reviewed");
        let mut q = f.queue();
        let r = q.preview(std::slice::from_ref(&p)).unwrap();
        let source = p.clone();
        let saved_copy = saved.clone();
        linux::RENAME_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&source, saved_copy).unwrap();
                fs::write(&source, b"unreviewed").unwrap();
                linux::RENAME_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move || {
                        fs::write(&source, b"new-occupant").unwrap();
                    }))
                });
            }))
        });
        let err = q.commit(&r.id, true, &Cancellation::default()).unwrap_err();
        let Error::Recovery { original, retained } = err else {
            panic!("expected Recovery");
        };
        assert_eq!(original, p);
        assert_eq!(fs::read(&p).unwrap(), b"new-occupant");
        assert_eq!(fs::read(retained).unwrap(), b"unreviewed");
        assert_eq!(fs::read(saved).unwrap(), b"reviewed");
        assert!(q.history().is_empty());
    }
    #[test]
    fn leaf_symlink_race_is_not_followed_and_is_rolled_back() {
        let f = Fixture::new();
        let p = f.file("reviewed", b"reviewed");
        let target = f.file("target", b"outside-selection");
        let saved = f.root.path().join("saved-reviewed");
        let mut q = f.queue();
        let r = q.preview(std::slice::from_ref(&p)).unwrap();
        let source = p.clone();
        let target_copy = target.clone();
        linux::RENAME_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&source, saved).unwrap();
                symlink(target_copy, &source).unwrap();
            }))
        });
        assert!(q.commit(&r.id, true, &Cancellation::default()).is_err());
        assert_eq!(fs::read(target).unwrap(), b"outside-selection");
        assert!(fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
    }
    #[test]
    fn restore_leaf_replacement_race_is_rolled_back() {
        let f = Fixture::new();
        let p = f.file("reviewed", b"reviewed");
        let mut q = f.queue();
        let r = q.preview(std::slice::from_ref(&p)).unwrap();
        let h = q
            .commit(&r.id, true, &Cancellation::default())
            .unwrap()
            .remove(0);
        let source = h.trashed.clone();
        let saved = f.trash.path().join("saved-reviewed");
        let saved_copy = saved.clone();
        linux::RENAME_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&source, saved_copy).unwrap();
                fs::write(&source, b"unreviewed").unwrap();
            }))
        });
        assert!(q.restore(&h.id, &Cancellation::default()).is_err());
        assert!(!Path::new(&p).exists());
        assert_eq!(fs::read(&h.trashed).unwrap(), b"unreviewed");
        assert_eq!(fs::read(saved).unwrap(), b"reviewed");
        assert!(!q.history()[0].restored);
    }
    #[test]
    fn cancellation_after_first_move_preserves_partial_history_and_consumes_review() {
        let f = Fixture::new();
        let a = f.file("a", b"a");
        let b = f.file("b", b"b");
        let mut q = f.queue();
        let r = q.preview(&[a.clone(), b.clone()]).unwrap();
        let cancel = Cancellation::default();
        let clone = cancel.clone();
        linux::RENAME_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || clone.cancel())));
        assert!(matches!(
            q.commit(&r.id, true, &cancel),
            Err(Error::Cancelled)
        ));
        assert!(!Path::new(&a).exists());
        assert!(Path::new(&b).exists());
        assert_eq!(q.history().len(), 1);
        assert!(matches!(
            q.commit(&r.id, true, &Cancellation::default()),
            Err(Error::UnknownReview)
        ));
        let id = q.history()[0].id.clone();
        q.restore(&id, &Cancellation::default()).unwrap();
        assert_eq!(fs::read(a).unwrap(), b"a");
    }
    #[test]
    fn destination_collision_never_overwrites() {
        let f = Fixture::new();
        let a = f.file("a", b"a");
        let b = f.file("b", b"b");
        let from = linux::open_dir(f.root.path()).unwrap();
        assert!(linux::rename(&from, "a", &from, "b").is_err());
        assert_eq!(fs::read(a).unwrap(), b"a");
        assert_eq!(fs::read(b).unwrap(), b"b");
    }
}
