use crate::{
    linux::{self, Dir, Identity},
    path_text, sensitive, Cancellation, Entry, Error, Limits, Result, Scan, ScanError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::Path,
};

#[cfg(test)]
thread_local! {
    static HASH_READ_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static SCAN_ENTRY_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

struct Frame {
    path: String,
    dir: File,
    iter: Dir,
    depth: usize,
}
fn scan_error(scan: &mut Scan, limits: &Limits, path: String, message: String) -> bool {
    if scan.errors.len() >= limits.max_errors {
        scan.truncated = true;
        return false;
    }
    scan.errors.push(ScanError { path, message });
    true
}
fn add(scan: &mut Scan, p: String, m: &std::fs::Metadata, seen: &mut HashSet<(u64, u64)>) {
    let e = linux::entry(p, m, !seen.insert((m.dev(), m.ino())));
    scan.apparent = scan.apparent.saturating_add(e.apparent);
    scan.allocated = scan.allocated.saturating_add(e.allocated);
    scan.entries.push(e);
    #[cfg(test)]
    SCAN_ENTRY_HOOK.with(|hook| {
        let action = hook.borrow_mut().take();
        if let Some(action) = action {
            action();
        }
    });
}
pub fn scan(root: &Path, limits: &Limits, cancel: &Cancellation) -> Result<Scan> {
    limits.validate()?;
    let root = path_text(root)?;
    if root.len() > limits.max_path_bytes {
        return Err(Error::Limit("root path".into()));
    }
    let mut out = Scan {
        root: root.clone(),
        entries: vec![],
        errors: vec![],
        apparent: 0,
        allocated: 0,
        cancelled: false,
        max_concurrency: 1,
        truncated: false,
    };
    if cancel.is_cancelled() {
        out.cancelled = true;
        return Ok(out);
    }
    // Legacy accepts regular-file roots too. A leaf symlink is always rejected.
    let base = if root == "/" {
        linux::open_dir(Path::new(&root))?
    } else {
        let (parent, name) = linux::parent(Path::new(&root))?;
        linux::open_at(&parent, &name, libc::O_PATH, 0)?
    };
    let meta = base.metadata()?;
    if meta.file_type().is_symlink() {
        return Err(Error::InvalidPath("Root cannot be a symlink".into()));
    }
    let mut seen = HashSet::new();
    add(&mut out, root.clone(), &meta, &mut seen);
    if !meta.is_dir() {
        out.cancelled = cancel.is_cancelled();
        return Ok(out);
    }
    let dir = linux::open_dir(Path::new(&root))?;
    if !Identity::of(&meta).same_object(&Identity::of(&dir.metadata()?)) {
        return Err(Error::Changed(root));
    }
    let iter = Dir::new(linux::open_at(
        &dir,
        ".",
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
    )?)?;
    let mut stack = vec![Frame {
        path: root,
        dir,
        iter,
        depth: 0,
    }];
    while let Some(frame) = stack.last_mut() {
        if cancel.is_cancelled() {
            out.cancelled = true;
            break;
        }
        if out.entries.len() >= limits.max_entries {
            out.truncated = true;
            break;
        }
        let name = match frame.iter.next() {
            Ok(Some(n)) => n,
            Ok(None) => {
                stack.pop();
                continue;
            }
            Err(e) => {
                let p = frame.path.clone();
                stack.pop();
                if !scan_error(&mut out, limits, p, e.to_string()) {
                    break;
                }
                continue;
            }
        };
        let Some(name) = name.to_str() else {
            if !scan_error(
                &mut out,
                limits,
                frame.path.clone(),
                "Non-UTF-8 name skipped".into(),
            ) {
                break;
            }
            continue;
        };
        let p = if frame.path == "/" {
            format!("/{name}")
        } else {
            format!("{}/{name}", frame.path)
        };
        if p.len() > limits.max_path_bytes {
            out.truncated = true;
            if !scan_error(
                &mut out,
                limits,
                frame.path.clone(),
                "Path limit skipped".into(),
            ) {
                break;
            }
            continue;
        }
        let m = match linux::stat_at(&frame.dir, name) {
            Ok(m) => m,
            Err(e) => {
                if !scan_error(&mut out, limits, p, e.to_string()) {
                    break;
                }
                continue;
            }
        };
        if m.dev() != meta.dev() {
            if !scan_error(&mut out, limits, p, "Mount boundary skipped".into()) {
                break;
            }
            continue;
        }
        let shared = seen.contains(&(m.dev(), m.ino()));
        add(&mut out, p.clone(), &m, &mut seen);
        if !m.is_dir() || shared {
            continue;
        }
        let depth = frame.depth + 1;
        if depth > limits.max_depth {
            out.truncated = true;
            if !scan_error(&mut out, limits, p, "Depth limit skipped".into()) {
                break;
            }
            continue;
        }
        let next = (|| {
            let dir = linux::open_at(&frame.dir, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
            if !Identity::of(&m).same_object(&Identity::of(&dir.metadata()?)) {
                return Err(Error::Changed(p.clone()));
            }
            let iter = Dir::new(linux::open_at(
                &dir,
                ".",
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?)?;
            Ok(Frame {
                path: p.clone(),
                dir,
                iter,
                depth,
            })
        })();
        match next {
            Ok(f) => stack.push(f),
            Err(e) => {
                if !scan_error(&mut out, limits, p, e.to_string()) {
                    break;
                }
            }
        }
    }
    out.cancelled |= cancel.is_cancelled();
    Ok(out)
}
pub fn duplicates(
    entries: &[Entry],
    limits: &Limits,
    cancel: &Cancellation,
) -> Result<Vec<Vec<String>>> {
    limits.validate()?;
    cancel.check()?;
    if entries.len() > limits.max_entries {
        return Err(Error::Limit("duplicate input entries".into()));
    }
    let mut sizes: BTreeMap<u64, Vec<&Entry>> = BTreeMap::new();
    let mut paths = HashSet::new();
    for e in entries {
        cancel.check()?;
        if e.path.len() > limits.max_path_bytes {
            return Err(Error::Limit("duplicate path bytes".into()));
        }
        if e.kind == "file" && !e.shared {
            if !paths.insert(&e.path) {
                return Err(Error::InvalidPath("duplicate input path".into()));
            }
            sizes.entry(e.apparent).or_default().push(e);
        }
    }
    let mut output = Vec::new();
    let mut budget = 0_u64;
    let mut buffer = [0_u8; 65536];
    for group in sizes.values().filter(|g| g.len() > 1) {
        let mut hashes: BTreeMap<[u8; 32], Vec<String>> = BTreeMap::new();
        for e in group {
            cancel.check()?;
            if sensitive(Path::new(&e.path)) {
                return Err(Error::Protected(e.path.clone()));
            }
            if e.apparent > limits.max_file_hash_bytes {
                return Err(Error::Limit("file hash bytes".into()));
            }
            budget = budget
                .checked_add(e.apparent)
                .ok_or_else(|| Error::Limit("hash bytes".into()))?;
            if budget > limits.max_hash_bytes {
                return Err(Error::Limit("total hash bytes".into()));
            }
            let (parent, name) = linux::parent(Path::new(&e.path))?;
            let mut file = linux::open_at(&parent, &name, libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
            let m = file.metadata()?;
            let current = linux::entry(e.path.clone(), &m, false);
            if !m.is_file()
                || current.dev != e.dev
                || current.ino != e.ino
                || current.apparent != e.apparent
                || current.mtime != e.mtime
            {
                return Err(Error::Changed(e.path.clone()));
            }
            let before = Identity::of(&m);
            let mut hash = Sha256::new();
            let mut read = 0_u64;
            loop {
                cancel.check()?;
                // Read at most the reviewed size + one byte to detect growth without
                // reading an unbounded stream if a file grows concurrently.
                let remaining = e.apparent.saturating_sub(read).saturating_add(1);
                let take = buffer.len().min(remaining as usize);
                let n = file.read(&mut buffer[..take])?;
                #[cfg(test)]
                HASH_READ_HOOK.with(|hook| {
                    let action = hook.borrow_mut().take();
                    if let Some(action) = action {
                        action();
                    }
                });
                if n == 0 {
                    break;
                }
                read += n as u64;
                if read > e.apparent {
                    return Err(Error::Changed(e.path.clone()));
                }
                hash.update(&buffer[..n]);
            }
            if read != e.apparent
                || before != Identity::of(&file.metadata()?)
                || before != Identity::of(&linux::stat_at(&parent, &name)?)
            {
                return Err(Error::Changed(e.path.clone()));
            }
            hashes
                .entry(hash.finalize().into())
                .or_default()
                .push(e.path.clone());
        }
        for mut group in hashes.into_values().filter(|g| g.len() > 1) {
            group.sort();
            output.push(group);
        }
    }
    output.sort();
    cancel.check()?;
    Ok(output)
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    #[test]
    fn hash_cancelled_after_first_chunk_returns_no_partial_groups() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/generated");
        fs::create_dir_all(&base).unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("mem-core-fixture-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(base)
            .unwrap();
        fs::write(fixture.path().join("a"), vec![42; 131072]).unwrap();
        fs::write(fixture.path().join("b"), vec![42; 131072]).unwrap();
        let entries = scan(fixture.path(), &Limits::default(), &Cancellation::default())
            .unwrap()
            .entries;
        let cancel = Cancellation::default();
        let clone = cancel.clone();
        HASH_READ_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || clone.cancel())));
        assert!(matches!(
            duplicates(&entries, &Limits::default(), &cancel),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn scan_cancelled_during_traversal_returns_partial_dto() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/generated");
        fs::create_dir_all(&base).unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("mem-core-fixture-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(base)
            .unwrap();
        fs::write(fixture.path().join("a"), b"a").unwrap();
        let cancel = Cancellation::default();
        let clone = cancel.clone();
        SCAN_ENTRY_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || clone.cancel())));
        let out = scan(fixture.path(), &Limits::default(), &cancel).unwrap();
        assert!(out.cancelled);
        assert_eq!(out.entries.len(), 1);
    }
}
