# mem-core v0.1 public contract

Standalone crate: `cargo test --manifest-path crates/mem-core/Cargo.toml --locked`.
Declared minimum Rust 1.89; validated toolchain Rust 1.99.0. Cargo.lock is committed.
Linux only for filesystem operations; other platforms return `UnsupportedPlatform`.
No root workspace or UI changes. No HTTP, Tauri commands, database, or background runtime.
Host adapters should execute blocking calls on a worker thread, retain Queue behind a mutex,
and expose cancellation with the cloneable atomic `Cancellation` token.

## Rust API (integration contract)

```rust
pub fn scan(root: &Path, limits: &Limits, cancel: &Cancellation) -> Result<Scan>;
pub fn duplicates(entries: &[Entry], limits: &Limits, cancel: &Cancellation)
    -> Result<Vec<Vec<String>>>;
pub struct Cancellation; // Default + Clone; cancel(), is_cancelled()
pub struct QueueOptions {
    pub root: PathBuf, // explicit allowed cleanup scope; root itself cannot be moved
    pub trash: PathBuf, // existing directory on same filesystem, outside root
    pub protected_paths: Vec<PathBuf>, // additions to built-in protections
    pub fixture: bool, // true ONLY for disposable mem-core-fixture-* test roots; skips git guard
}
impl Queue {
    pub fn new(options: QueueOptions) -> Result<Self>;
    pub fn preview(&mut self, paths: &[String]) -> Result<Preview>;
    pub fn commit(&mut self, id: &str, confirmed: bool, cancel: &Cancellation)
        -> Result<Vec<History>>;
    pub fn restore(&mut self, id: &str, cancel: &Cancellation) -> Result<Vec<History>>;
    pub fn history(&self) -> &[History];
    pub fn discard_preview(&mut self, id: &str) -> bool;
}
```

Entry JSON matches legacy: `{path,name,kind,apparent,allocated,mtime,dev,ino,shared}`.
`kind`: `file | directory | symlink | other`; mtime is milliseconds since Unix epoch (f64).
Byte counts/dev/ino are u64 JSON numbers. JS consumers must preserve integers above
Number.MAX_SAFE_INTEGER (use a bigint-aware JSON parser or bridge serialization).
Scan JSON: `{root,entries,errors:[{path,message}],apparent,allocated,cancelled,
maxConcurrency,truncated}`. `maxConcurrency` is 1. `truncated` is additive and MUST
be displayed: totals then represent a partial scan. Hard links count once.
Preview JSON: `{id,items,created}`; created is Unix milliseconds. UUIDs opaque.
History JSON: `{id,original,trashed,info,dev,ino,restored}`; restored defaults false.
DTOs derive Serialize/Deserialize, fields use legacy camelCase where applicable.
Queue never trusts deserialized DTOs: reviews and history are kept internally.

Limits fields: `max_entries` (100000), `max_errors` (1000), `max_depth` (128),
`max_path_bytes` (4096), `max_hash_bytes` (1 GiB across the call),
`max_file_hash_bytes` (256 MiB). Hard caps are 100000 entries, 1000 errors, 128 depth, 4096 path bytes,
8 GiB total hash bytes and 1 GiB per-file hash bytes; zero is rejected.
Traversal streams directory names with bounded depth/FDs and stops at entry/output cap.
Duplicate grouping uses SHA-256, bounded 64 KiB read buffers, rejects protected content,
checks identity/size/timestamps before and after reads, and returns Err on incomplete work.
Cancellation scan returns partial Scan with cancelled=true; hash/queue returns Cancelled.
Cancellation is checked between entries and hash chunks; a blocking filesystem syscall
cannot be interrupted. Scan has per-path bounded errors and never follows symlinks,
including ancestors; different-device mount boundaries are skipped (same-device bind
mounts are not distinguished). Directory and regular-file roots are supported; symlink
roots are rejected. Enumeration order follows the filesystem; duplicate groups are sorted. Paths must be absolute, normalized,
UTF-8; non-UTF-8 child names are skipped with errors, never lossy cleanup paths.

Queue caps: 500 distinct files per review, 32 live reviews, 10000 history records.
Only user-owned regular files with one hard link may be moved. Cleanup root and every
source parent within it must be user-owned and not writable by group or others. Sensitive names, home/system roots,
caller protected ancestors/descendants and active git trees are protected. Trash and
root must exist, never be symlinks, and are pinned by descriptors. Trash must be owned
by the effective user with mode 0700 (an unusually restrictive umask may require the
host to provision subdirectories itself); `files` and `info` are created 0700 when absent.
No cleanup directory creation outside the supplied Trash. No cross-device copy fallback.
`commit` requires an existing review ID and confirmed=true; previewed identities include
inode/device, size, mtime/ctime nanoseconds, mode/link count internally. Preflight checks
all entries before any move. A consumed review cannot be replayed. Operations are
sequential, not batch-atomic. On cancellation/error, query history for completed moves.

Linux primitives: component-by-component O_NOFOLLOW directory opens, O_PATH|O_NOFOLLOW metadata descriptors with fstat, renameat2(RENAME_NOREPLACE), exclusive nofollow metadata creation.
An entry is revalidated just before rename and in its destination. Linux lacks an atomic
"rename only if inode matches" primitive: a concurrent leaf replacement can be briefly
moved, then rolled back with NOREPLACE. If rollback cannot restore the original name,
Error::Recovery carries original and retained paths. Nothing is overwritten or permanently
deleted; unexpected objects are retained for manual recovery. Parent descriptors anchor
operations even if ancestors are renamed. Same-user malicious namespace manipulation
cannot be fully prevented; host must not treat this as a sandbox.

Trash writes freedesktop `.trashinfo` with percent-encoded original UTF-8 path and local
DeletionDate. Data filenames are bare UUIDs to avoid NAME_MAX overflow; display the
original basename from History.original. Restore validates pinned trashed identity and original parent identity,
then renames with NOREPLACE. Restore never overwrites an existing destination, follows
symlinks, or creates missing parents. Metadata is retained after restore as an audit
artifact (history.restored=true); no purge API. History and reviews are in memory only;
restart recovery uses retained Trash data, no history import in this version. Failed
commits may leave an orphan .trashinfo file; these are retained for manual audit.
Fixture mode requires a root basename beginning mem-core-fixture- and only skips the git
and current-working-directory protections; it is never appropriate for owner data.

Errors: `Io(String)`, `InvalidPath(String)`, `Protected(String)`, `Changed(String)`,
`Limit(String)`, `Cancelled`, `UnknownReview`, `UnknownHistory`, `ConfirmationRequired`,
`UnsupportedPlatform`, `Recovery { original: String, retained: String }`.
Errors implement Display/std::error::Error, not a wire error DTO; host maps to its envelope.
