#![cfg(target_os = "linux")]
use mem_core::{duplicates, scan, Cancellation, Error, Limits, Queue, QueueOptions};
use std::{
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{symlink, PermissionsExt},
    },
    path::{Path, PathBuf},
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
        Self {
            root: Builder::new()
                .prefix("mem-core-fixture-")
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(&base)
                .unwrap(),
            trash: Builder::new()
                .prefix("mem-core-trash-fixture-")
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(&base)
                .unwrap(),
        }
    }
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.root.path().join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().into()
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
    fn scan(&self) -> mem_core::Scan {
        scan(
            self.root.path(),
            &Limits::default(),
            &Cancellation::default(),
        )
        .unwrap()
    }
}
#[test]
fn totals_sparse_hardlinks_and_symlinks() {
    let f = Fixture::new();
    let a = f.file("a", b"payload");
    fs::hard_link(&a, f.root.path().join("hardlink")).unwrap();
    symlink(&a, f.root.path().join("symbolic")).unwrap();
    let sparse = fs::File::create(f.root.path().join("sparse")).unwrap();
    sparse.set_len(16 << 20).unwrap();
    let out = f.scan();
    assert_eq!(out.entries.len(), 5);
    assert_eq!(out.entries.iter().filter(|e| e.shared).count(), 1);
    let e = out.entries.iter().find(|e| e.name == "sparse").unwrap();
    assert!(e.apparent > e.allocated);
    assert_eq!(
        out.apparent,
        out.entries.iter().map(|e| e.apparent).sum::<u64>()
    );
    assert_eq!(
        out.allocated,
        out.entries.iter().map(|e| e.allocated).sum::<u64>()
    );
    assert!(out.errors.is_empty());
    assert!(!out.truncated);
}
#[test]
fn duplicate_groups_distinguish_content_and_ignore_hardlinks() {
    let f = Fixture::new();
    let a = f.file("a", b"same");
    let b = f.file("b", b"same");
    f.file("c", b"diff");
    fs::hard_link(&a, f.root.path().join("hard")).unwrap();
    let groups = duplicates(
        &f.scan().entries,
        &Limits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 2);
    assert!(groups[0].contains(&b));
    // Directory enumeration can choose either alias as the first counted inode.
    assert!(
        groups[0].contains(&a)
            || groups[0].contains(&f.root.path().join("hard").to_str().unwrap().into())
    );
}
#[test]
fn empty_duplicates() {
    let f = Fixture::new();
    f.file("a", b"");
    f.file("b", b"");
    assert_eq!(
        duplicates(
            &f.scan().entries,
            &Limits::default(),
            &Cancellation::default()
        )
        .unwrap()
        .len(),
        1
    );
}
#[test]
fn scan_never_follows_external_symlink() {
    let f = Fixture::new();
    let outside = Fixture::new();
    outside.file("unseen", b"external");
    symlink(outside.root.path(), f.root.path().join("link")).unwrap();
    let out = f.scan();
    assert_eq!(out.entries.len(), 2);
    assert_eq!(out.entries[1].kind, "symlink");
    assert!(scan(
        &f.root.path().join("link"),
        &Limits::default(),
        &Cancellation::default()
    )
    .is_err());
}
#[test]
fn reject_symlink_ancestor_and_noncanonical_paths() {
    let f = Fixture::new();
    let alias = f.trash.path().join("alias");
    symlink(f.root.path(), &alias).unwrap();
    assert!(scan(&alias, &Limits::default(), &Cancellation::default()).is_err());
    for p in [
        PathBuf::from("relative"),
        f.root.path().join(".."),
        PathBuf::from(format!("{}/./x", f.root.path().display())),
    ] {
        assert!(scan(&p, &Limits::default(), &Cancellation::default()).is_err());
    }
}
#[test]
fn scan_entry_depth_path_and_error_limits() {
    let f = Fixture::new();
    f.file("a", b"a");
    f.file("b", b"b");
    let mut limits = Limits {
        max_entries: 2,
        ..Limits::default()
    };
    let out = scan(f.root.path(), &limits, &Cancellation::default()).unwrap();
    assert_eq!(out.entries.len(), 2);
    assert!(out.truncated);
    fs::create_dir_all(f.root.path().join("one/two/three")).unwrap();
    limits.max_entries = 100;
    limits.max_depth = 1;
    let out = scan(f.root.path(), &limits, &Cancellation::default()).unwrap();
    assert!(out.truncated);
    assert!(out.entries.iter().all(|e| !e.path.ends_with("three")));
    limits.max_depth = 128;
    limits.max_path_bytes = f.root.path().to_str().unwrap().len() + 1;
    limits.max_errors = 1;
    let out = scan(f.root.path(), &limits, &Cancellation::default()).unwrap();
    assert!(out.truncated);
    assert!(out.errors.len() <= 1);
    limits.max_entries = 0;
    assert!(scan(f.root.path(), &limits, &Cancellation::default()).is_err());
}
#[test]
fn non_utf8_names_are_not_lossy() {
    let f = Fixture::new();
    fs::write(
        f.root.path().join(std::ffi::OsString::from_vec(vec![0xff])),
        b"x",
    )
    .unwrap();
    let out = f.scan();
    assert_eq!(out.entries.len(), 1);
    assert_eq!(out.errors.len(), 1);
}
#[test]
fn cancelled_scan_and_duplicates() {
    let f = Fixture::new();
    f.file("a", b"a");
    let cancel = Cancellation::default();
    let clone = cancel.clone();
    clone.cancel();
    let out = scan(f.root.path(), &Limits::default(), &cancel).unwrap();
    assert!(out.cancelled);
    assert!(out.entries.is_empty());
    assert!(matches!(
        duplicates(&f.scan().entries, &Limits::default(), &cancel),
        Err(Error::Cancelled)
    ));
}
#[test]
fn changed_hash_identity_and_protected_names_fail_closed() {
    let f = Fixture::new();
    let a = f.file("a", b"same");
    f.file("b", b"same");
    let out = f.scan();
    fs::rename(&a, f.root.path().join("old")).unwrap();
    fs::write(&a, b"same").unwrap();
    assert!(matches!(
        duplicates(&out.entries, &Limits::default(), &Cancellation::default()),
        Err(Error::Changed(_))
    ));
    let f = Fixture::new();
    f.file(".env.production", b"fake");
    f.file("b", b"fake");
    assert!(matches!(
        duplicates(
            &f.scan().entries,
            &Limits::default(),
            &Cancellation::default()
        ),
        Err(Error::Protected(_))
    ));
}
#[test]
fn hash_byte_and_input_limits() {
    let f = Fixture::new();
    f.file("a", b"same");
    f.file("b", b"same");
    let out = f.scan();
    for limits in [
        Limits {
            max_hash_bytes: 7,
            ..Limits::default()
        },
        Limits {
            max_file_hash_bytes: 3,
            ..Limits::default()
        },
        Limits {
            max_entries: 1,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            duplicates(&out.entries, &limits, &Cancellation::default()),
            Err(Error::Limit(_))
        ));
    }
    let e = out
        .entries
        .iter()
        .find(|e| e.kind == "file")
        .unwrap()
        .clone();
    assert!(duplicates(
        &[e.clone(), e],
        &Limits::default(),
        &Cancellation::default()
    )
    .is_err());
}
#[test]
fn preview_commit_restore_end_to_end_and_dto() {
    let f = Fixture::new();
    let p = f.file("space % 雪.txt", b"reviewed");
    let mut q = f.queue();
    let review = q.preview(std::slice::from_ref(&p)).unwrap();
    assert_eq!(review.items[0].path, p);
    assert!(matches!(
        q.commit(&review.id, false, &Cancellation::default()),
        Err(Error::ConfirmationRequired)
    ));
    let h = q
        .commit(&review.id, true, &Cancellation::default())
        .unwrap()
        .remove(0);
    assert!(!Path::new(&p).exists());
    assert_eq!(fs::read(&h.trashed).unwrap(), b"reviewed");
    let info = fs::read_to_string(&h.info).unwrap();
    assert!(info.contains("space%20%25%20%E9%9B%AA.txt"));
    assert!(info.contains("DeletionDate="));
    assert!(matches!(
        q.commit(&review.id, true, &Cancellation::default()),
        Err(Error::UnknownReview)
    ));
    let restored = q.restore(&h.id, &Cancellation::default()).unwrap();
    assert!(restored[0].restored);
    assert_eq!(fs::read(&p).unwrap(), b"reviewed");
    assert!(!Path::new(&h.trashed).exists());
    assert!(Path::new(&h.info).exists());
    assert!(matches!(
        q.restore(&h.id, &Cancellation::default()),
        Err(Error::UnknownHistory)
    ));
    let json = serde_json::to_value(f.scan()).unwrap();
    assert_eq!(json["maxConcurrency"], 1);
    assert!(json.get("max_concurrency").is_none());
    assert!(serde_json::to_value(&review)
        .unwrap()
        .get("created")
        .is_some());
}
#[test]
fn batch_stale_preflight_does_not_move_anything() {
    let f = Fixture::new();
    let a = f.file("a", b"a");
    let b = f.file("b", b"b");
    let mut q = f.queue();
    let review = q.preview(&[a.clone(), b.clone()]).unwrap();
    fs::write(&b, b"modified").unwrap();
    assert!(matches!(
        q.commit(&review.id, true, &Cancellation::default()),
        Err(Error::Changed(_))
    ));
    assert!(Path::new(&a).exists());
    assert!(q.history().is_empty());
}
#[test]
fn replacement_symlink_after_review_preserves_outside_file() {
    let f = Fixture::new();
    let p = f.file("a", b"a");
    let outside = Fixture::new();
    let other = outside.file("outside", b"outside");
    let mut q = f.queue();
    let r = q.preview(std::slice::from_ref(&p)).unwrap();
    fs::rename(&p, f.root.path().join("old")).unwrap();
    symlink(&other, &p).unwrap();
    assert!(q.commit(&r.id, true, &Cancellation::default()).is_err());
    assert_eq!(fs::read(other).unwrap(), b"outside");
    assert!(q.history().is_empty());
}
#[test]
fn restore_collision_preserves_both_objects() {
    let f = Fixture::new();
    let p = f.file("a", b"reviewed");
    let mut q = f.queue();
    let r = q.preview(std::slice::from_ref(&p)).unwrap();
    let h = q
        .commit(&r.id, true, &Cancellation::default())
        .unwrap()
        .remove(0);
    fs::write(&p, b"replacement").unwrap();
    assert!(q.restore(&h.id, &Cancellation::default()).is_err());
    assert_eq!(fs::read(&p).unwrap(), b"replacement");
    assert_eq!(fs::read(&h.trashed).unwrap(), b"reviewed");
    assert!(!q.history()[0].restored);
}
#[test]
fn changed_trash_and_changed_original_parent_rejected() {
    let f = Fixture::new();
    fs::create_dir(f.root.path().join("sub")).unwrap();
    let p = f.file("sub/a", b"a");
    let mut q = f.queue();
    let r = q.preview(std::slice::from_ref(&p)).unwrap();
    let h = q
        .commit(&r.id, true, &Cancellation::default())
        .unwrap()
        .remove(0);
    fs::rename(f.root.path().join("sub"), f.root.path().join("old-sub")).unwrap();
    fs::create_dir(f.root.path().join("sub")).unwrap();
    assert!(q.restore(&h.id, &Cancellation::default()).is_err());
    assert!(Path::new(&h.trashed).exists());
    fs::write(&h.trashed, b"changed").unwrap();
    assert!(q.restore(&h.id, &Cancellation::default()).is_err());
}
#[test]
fn reject_hardlinks_directories_protected_paths_and_selection() {
    let f = Fixture::new();
    let p = f.file("a", b"a");
    fs::hard_link(&p, f.root.path().join("alias")).unwrap();
    let mut q = f.queue();
    assert!(q.preview(&[p]).is_err());
    assert!(q
        .preview(&[f.root.path().to_str().unwrap().into()])
        .is_err());
    assert!(q.preview(&[]).is_err());
    let p = f.file("auth.json", b"fixture fake");
    assert!(q.preview(&[p]).is_err());
    let p = f.file("plain", b"plain");
    assert!(q.preview(&[p.clone(), p]).is_err());
    let outside = f.trash.path().join("outside");
    fs::write(&outside, b"x").unwrap();
    assert!(q.preview(&[outside.to_str().unwrap().into()]).is_err());
}
#[test]
fn trash_symlink_permissions_and_root_replacement_fail_closed() {
    let f = Fixture::new();
    fs::set_permissions(f.trash.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Queue::new(QueueOptions {
        root: f.root.path().into(),
        trash: f.trash.path().into(),
        protected_paths: vec![],
        fixture: true
    })
    .is_err());
    fs::set_permissions(f.trash.path(), fs::Permissions::from_mode(0o700)).unwrap();
    symlink(f.root.path(), f.trash.path().join("files")).unwrap();
    assert!(Queue::new(QueueOptions {
        root: f.root.path().into(),
        trash: f.trash.path().into(),
        protected_paths: vec![],
        fixture: true
    })
    .is_err());
    let f = Fixture::new();
    let p = f.file("a", b"a");
    let mut q = f.queue();
    let r = q.preview(&[p]).unwrap();
    let root = f.root.path();
    let moved = root.with_extension("saved");
    fs::rename(root, &moved).unwrap();
    fs::create_dir(root).unwrap();
    assert!(q.commit(&r.id, true, &Cancellation::default()).is_err());
    fs::remove_dir(root).unwrap();
    fs::rename(moved, root).unwrap();
}
#[test]
fn git_guard_fixture_scope_and_review_cap() {
    let f = Fixture::new();
    fs::create_dir(f.root.path().join(".git")).unwrap();
    assert!(Queue::new(QueueOptions {
        root: f.root.path().into(),
        trash: f.trash.path().into(),
        protected_paths: vec![],
        fixture: false
    })
    .is_err());
    let p = f.file("a", b"a");
    let mut q = f.queue();
    let mut ids = vec![];
    for _ in 0..32 {
        ids.push(q.preview(std::slice::from_ref(&p)).unwrap().id);
    }
    assert!(matches!(
        q.preview(std::slice::from_ref(&p)),
        Err(Error::Limit(_))
    ));
    assert!(q.discard_preview(&ids[0]));
    assert!(!q.discard_preview(&ids[0]));
    assert!(q.preview(&[p]).is_ok());
}
#[test]
fn cancelled_queue_does_not_mutate_or_consume_review() {
    let f = Fixture::new();
    let p = f.file("a", b"a");
    let mut q = f.queue();
    let r = q.preview(std::slice::from_ref(&p)).unwrap();
    let c = Cancellation::default();
    c.cancel();
    assert!(matches!(q.commit(&r.id, true, &c), Err(Error::Cancelled)));
    assert!(Path::new(&p).exists());
    assert!(q.history().is_empty());
    let h = q
        .commit(&r.id, true, &Cancellation::default())
        .unwrap()
        .remove(0);
    assert!(matches!(q.restore(&h.id, &c), Err(Error::Cancelled)));
    assert!(Path::new(&h.trashed).exists());
}
#[test]
fn trash_subdirectory_replacement_fails_closed() {
    let f = Fixture::new();
    let p = f.file("a", b"a");
    let mut q = f.queue();
    let r = q.preview(std::slice::from_ref(&p)).unwrap();
    fs::rename(
        f.trash.path().join("files"),
        f.trash.path().join("saved-files"),
    )
    .unwrap();
    fs::create_dir(f.trash.path().join("files")).unwrap();
    assert!(q.commit(&r.id, true, &Cancellation::default()).is_err());
    assert!(Path::new(&p).exists());
    assert!(q.history().is_empty());
}

#[test]
fn regular_file_root_matches_legacy_scan() {
    let f = Fixture::new();
    let p = f.file("file-root", b"root-data");
    let out = scan(Path::new(&p), &Limits::default(), &Cancellation::default()).unwrap();
    assert_eq!(out.entries.len(), 1);
    assert_eq!(out.entries[0].kind, "file");
    assert_eq!(out.apparent, 9);
    assert!(!out.truncated);
}

#[test]
fn ancestor_symlink_replacement_after_review_cannot_escape() {
    let f = Fixture::new();
    fs::create_dir(f.root.path().join("sub")).unwrap();
    let p = f.file("sub/a", b"reviewed");
    let mut q = f.queue();
    let r = q.preview(std::slice::from_ref(&p)).unwrap();
    let outside = Fixture::new();
    outside.file("a", b"outside");
    fs::rename(f.root.path().join("sub"), f.root.path().join("saved")).unwrap();
    symlink(outside.root.path(), f.root.path().join("sub")).unwrap();
    assert!(q.commit(&r.id, true, &Cancellation::default()).is_err());
    assert_eq!(fs::read(outside.root.path().join("a")).unwrap(), b"outside");
    assert_eq!(
        fs::read(f.root.path().join("saved/a")).unwrap(),
        b"reviewed"
    );
    assert!(q.history().is_empty());
}

#[test]
fn externally_writable_cleanup_parents_are_rejected() {
    let f = Fixture::new();
    fs::create_dir(f.root.path().join("sub")).unwrap();
    let p = f.file("sub/a", b"a");
    let mut q = f.queue();
    fs::set_permissions(f.root.path().join("sub"), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(q.preview(&[p]), Err(Error::Protected(_))));
    fs::set_permissions(f.root.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(Queue::new(QueueOptions {
        root: f.root.path().into(),
        trash: f.trash.path().into(),
        protected_paths: vec![],
        fixture: true
    })
    .is_err());
}
