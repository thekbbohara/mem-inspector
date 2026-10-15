//! All unsafe code is confined to descriptor-relative syscall wrappers.
use crate::{path_text, Entry, Error, Result};
use std::{
    ffi::{CStr, CString, OsString},
    fs::{File, Metadata},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStringExt, fs::MetadataExt},
    },
    path::Path,
};

fn cstr(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::InvalidPath(s.into()))
}
fn file_from_fd(fd: i32) -> Result<File> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful openat returns a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(crate) fn open_at(dir: &File, name: &str, flags: i32, mode: u32) -> Result<File> {
    let name = cstr(name)?;
    // SAFETY: valid directory fd and NUL-terminated string; result ownership transferred.
    file_from_fd(unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    })
}
pub(crate) fn open_dir(p: &Path) -> Result<File> {
    let text = path_text(p)?;
    let slash = cstr("/")?;
    // SAFETY: constant absolute directory, open returns new owned fd.
    let mut dir = file_from_fd(unsafe {
        libc::open(
            slash.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?;
    for part in text[1..].split('/').filter(|x| !x.is_empty()) {
        dir = open_at(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    }
    Ok(dir)
}
pub(crate) fn parent(p: &Path) -> Result<(File, String)> {
    path_text(p)?;
    let name = p
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::InvalidPath(p.display().to_string()))?;
    Ok((
        open_dir(
            p.parent()
                .ok_or_else(|| Error::InvalidPath("no parent".into()))?,
        )?,
        name.into(),
    ))
}
pub(crate) fn stat_at(dir: &File, name: &str) -> Result<Metadata> {
    // O_PATH never reads content and can inspect a symlink without following it.
    open_at(dir, name, libc::O_PATH, 0)?
        .metadata()
        .map_err(Into::into)
}

pub(crate) fn user_owned(m: &Metadata) -> bool {
    // SAFETY: geteuid has no arguments or memory side effects.
    m.uid() == unsafe { libc::geteuid() }
}
pub(crate) fn owned_no_external_write(dir: &File) -> Result<bool> {
    let m = dir.metadata()?;
    Ok(m.is_dir() && user_owned(&m) && m.mode() & 0o022 == 0)
}
pub(crate) fn owned_private(dir: &File) -> Result<bool> {
    let m = dir.metadata()?;
    Ok(m.is_dir() && user_owned(&m) && m.mode() & 0o777 == 0o700)
}
pub(crate) fn exists_at(dir: &File, name: &str) -> Result<bool> {
    let name = cstr(name)?;
    // SAFETY: pinned directory fd and NUL-terminated name. O_PATH never follows symlinks.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd >= 0 {
        drop(file_from_fd(fd)?);
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(err.into())
    }
}
#[cfg(test)]
thread_local! { pub(crate) static RENAME_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }

pub(crate) fn mkdir_at(dir: &File, name: &str) -> Result<()> {
    let name = cstr(name)?;
    // SAFETY: pinned directory and valid name; no path traversal.
    let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(err.into());
        }
    }
    Ok(())
}
pub(crate) fn rename(from: &File, old: &str, to: &File, new: &str) -> Result<()> {
    let old = cstr(old)?;
    let new = cstr(new)?;
    #[cfg(test)]
    RENAME_HOOK.with(|hook| {
        let action = hook.borrow_mut().take();
        if let Some(action) = action {
            action();
        }
    });
    // SAFETY: pinned directory descriptors and valid names. NOREPLACE is atomic.
    let rc = unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            old.as_ptr(),
            to.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if matches!(
        err.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        return Err(crate::Error::UnsupportedPlatform);
    }
    Err(err.into())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: i64,
    pub ctime: i64,
    pub ctime_ns: i64,
    pub mode: u32,
    pub nlink: u64,
}
impl Identity {
    pub fn of(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
            mode: m.mode(),
            nlink: m.nlink(),
        }
    }
    pub fn same_object(&self, other: &Self) -> bool {
        self.dev == other.dev && self.ino == other.ino && self.mode == other.mode
    }
    // Renaming legitimately updates ctime; preserve all other reviewed attributes.
    pub fn after_move(&self, other: &Self) -> bool {
        self.same_object(other)
            && self.size == other.size
            && self.mtime == other.mtime
            && self.mtime_ns == other.mtime_ns
            && self.nlink == other.nlink
    }
}
pub(crate) fn entry(p: String, m: &Metadata, shared: bool) -> Entry {
    Entry {
        name: Path::new(&p)
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("/")
            .into(),
        path: p,
        kind: if m.file_type().is_symlink() {
            "symlink"
        } else if m.is_dir() {
            "directory"
        } else if m.is_file() {
            "file"
        } else {
            "other"
        }
        .into(),
        apparent: if shared { 0 } else { m.size() },
        allocated: if shared {
            0
        } else {
            m.blocks().saturating_mul(512)
        },
        mtime: m.mtime() as f64 * 1000.0 + m.mtime_nsec() as f64 / 1_000_000.0,
        dev: m.dev(),
        ino: m.ino(),
        shared,
    }
}
pub(crate) struct Dir(*mut libc::DIR);
impl Dir {
    pub fn new(file: File) -> Result<Self> {
        use std::os::fd::IntoRawFd;
        let fd = file.into_raw_fd();
        // SAFETY: fd is owned and readable directory descriptor. fdopendir takes it on success.
        let ptr = unsafe { libc::fdopendir(fd) };
        if ptr.is_null() {
            let err = std::io::Error::last_os_error();
            // SAFETY: fdopendir failed, so fd ownership remains here.
            unsafe {
                libc::close(fd);
            }
            return Err(err.into());
        }
        Ok(Self(ptr))
    }
    pub fn next(&mut self) -> Result<Option<OsString>> {
        loop {
            // SAFETY: live exclusively owned DIR, thread-local errno, and returned name is
            // copied before the next call. Linux dirent name is NUL-terminated by readdir.
            let ptr = unsafe {
                *libc::__errno_location() = 0;
                libc::readdir(self.0)
            };
            if ptr.is_null() {
                let err = std::io::Error::last_os_error();
                return if err.raw_os_error() == Some(0) {
                    Ok(None)
                } else {
                    Err(err.into())
                };
            }
            // SAFETY: non-null readdir result remains valid until next call.
            let bytes = unsafe { CStr::from_ptr((*ptr).d_name.as_ptr()).to_bytes() };
            if bytes != b"." && bytes != b".." {
                return Ok(Some(OsString::from_vec(bytes.into())));
            }
        }
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        // SAFETY: uniquely owned DIR pointer, close exactly once.
        unsafe {
            libc::closedir(self.0);
        }
    }
}
