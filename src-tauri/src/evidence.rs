//! Local cache-intent evidence. Reads only the standard marker header, never payloads.
use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    path::{Component, Path},
};
const HEADER: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";
fn open_at(dir: &File, name: &str, directory: bool) -> Option<File> {
    let name = CString::new(name).ok()?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: valid descriptor and NUL-terminated name; ownership transfers once.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    (fd >= 0).then(|| unsafe { File::from_raw_fd(fd) })
}
fn exists_at(dir: &File, name: &str) -> bool {
    let name = CString::new(name).expect("constant name");
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        ) == 0
    }
}
fn tagged(dir: &File) -> bool {
    let Some(mut file) = open_at(dir, "CACHEDIR.TAG", false) else {
        return false;
    };
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() >= HEADER.len() as u64)
    {
        return false;
    }
    let mut header = [0u8; 43];
    file.read_exact(&mut header).is_ok() && header == HEADER
}
/// A tag records cache intent, not an absolute safety guarantee. Git trees and
/// marker files themselves stay held. Every component is opened without following links.
pub fn cache_intent(root: &Path, path: &Path) -> bool {
    if path
        .file_name()
        .is_some_and(|n| n == "CACHEDIR.TAG" || n == ".mem-inspector-fixture")
    {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if !parent.starts_with(root) || !root.is_absolute() {
        return false;
    }
    let Ok(mut dir) = File::open("/") else {
        return false;
    };
    let mut current = std::path::PathBuf::from("/");
    let mut cache = false;
    for component in parent.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => {
                let Some(name) = name.to_str() else {
                    return false;
                };
                let Some(next) = open_at(&dir, name, true) else {
                    return false;
                };
                dir = next;
                current.push(name);
                if exists_at(&dir, ".git") {
                    return false;
                }
                if current.starts_with(root) && tagged(&dir) {
                    cache = true;
                }
            }
            _ => return false,
        }
    }
    cache
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_spoof_symlink_project_and_unknown_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let cache = root.join("cache");
        std::fs::create_dir(&cache).unwrap();
        let item = cache.join("payload");
        std::fs::write(&item, b"synthetic").unwrap();
        assert!(!cache_intent(root, &item));
        std::fs::write(
            cache.join("CACHEDIR.TAG"),
            b"Signature: not-the-standard-signature",
        )
        .unwrap();
        assert!(!cache_intent(root, &item));
        std::fs::write(cache.join("CACHEDIR.TAG"), HEADER).unwrap();
        assert!(cache_intent(root, &item));
        assert!(!cache_intent(root, &cache.join("CACHEDIR.TAG")));
        std::fs::write(cache.join(".git"), b"gitdir: elsewhere").unwrap();
        assert!(!cache_intent(root, &item));
        std::fs::remove_file(cache.join(".git")).unwrap();
        std::fs::rename(cache.join("CACHEDIR.TAG"), root.join("real-tag")).unwrap();
        std::os::unix::fs::symlink(root.join("real-tag"), cache.join("CACHEDIR.TAG")).unwrap();
        assert!(!cache_intent(root, &item));
        std::os::unix::fs::symlink(&cache, root.join("alias")).unwrap();
        assert!(!cache_intent(root, &root.join("alias/payload")));
        assert!(!cache_intent(root, &root.join("personal")));
    }
}
