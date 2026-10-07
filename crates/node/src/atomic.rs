//! Atomic file replacement (docs/design/24-self-healing.md, red team #5 and
//! #19): a key, share or network file is never seen half-written. The new
//! bytes go to a temporary file in the same directory, are synced, and are
//! renamed over the target — one directory-entry change, so a crash or a full
//! disk leaves either the whole old file or the whole new one, never a
//! truncated mix a later start would treat as data loss.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Replace `path` with `bytes`, atomically. `mode` is the new file's mode
/// (0o600 for secrets). The parent directory is synced too, so the rename
/// itself survives a power cut.
pub fn replace(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    write(path, bytes, mode, false).map_err(|e| e.to_string())
}

/// Create a secret once, atomically, without ever replacing an existing key.
/// The synced temporary inode is published only if `path` is absent.
pub fn create(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    create_once(path, bytes, mode).map_err(|e| e.to_string())
}

#[derive(Debug)]
pub enum CreateError {
    AlreadyExists,
    Storage(String),
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists => f.write_str("file already exists"),
            Self::Storage(e) => f.write_str(e),
        }
    }
}

impl From<String> for CreateError {
    fn from(e: String) -> Self { Self::Storage(e) }
}

/// Only an actual publication race is `AlreadyExists`. In particular, an
/// error after publication must never be mistaken for another creator winning.
pub fn create_once(path: &Path, bytes: &[u8], mode: u32) -> Result<(), CreateError> {
    write(path, bytes, mode, true)
}

pub fn sync_parent(path: &Path) -> Result<(), String> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    #[cfg(test)]
    if FAIL_SYNC.with(|p| p.borrow().as_deref() == Some(dir)) {
        return Err(format!("{}: injected directory fsync failure", dir.display()));
    }
    let d = std::fs::File::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    d.sync_all().map_err(|e| format!("{}: {e}", dir.display()))
}

#[cfg(test)]
thread_local! { static FAIL_SYNC: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
pub(crate) fn fail_sync_for_test(dir: Option<&Path>) {
    FAIL_SYNC.with(|p| *p.borrow_mut() = dir.map(Path::to_path_buf));
}

#[cfg(target_os = "macos")]
fn publish_once(tmp: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    unsafe extern "C" {
        fn renameatx_np(from_fd: libc::c_int, from: *const libc::c_char,
            to_fd: libc::c_int, to: *const libc::c_char, flags: libc::c_uint) -> libc::c_int;
    }
    let from = std::ffi::CString::new(tmp.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // macOS 14+ is our deployment floor. RENAME_EXCL is an atomic,
    // no-overwrite rename, so concurrent creators cannot replace a winner.
    if unsafe { renameatx_np(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn publish_once(tmp: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::hard_link(tmp, path)?;
    std::fs::remove_file(tmp)
}

fn write(path: &Path, bytes: &[u8], mode: u32, exclusive: bool) -> Result<(), CreateError> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.new-{}-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or_default(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let prepared = (|| -> Result<(), std::io::Error> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = prepared {
        let _ = std::fs::remove_file(&tmp);
        return Err(CreateError::Storage(format!("{}: {e}", tmp.display())));
    }
    let published = if exclusive {
        publish_once(&tmp, path)
    } else {
        std::fs::rename(&tmp, path)
    };
    if let Err(e) = published {
        if let Err(cleanup) = std::fs::remove_file(&tmp) {
            return Err(CreateError::Storage(format!("{}: {e}; temporary-file cleanup failed: {cleanup}", path.display())));
        }
        return Err(if exclusive && e.kind() == std::io::ErrorKind::AlreadyExists {
            CreateError::AlreadyExists
        } else {
            CreateError::Storage(format!("{}: {e}", path.display()))
        });
    }
    sync_parent(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aether-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_replaced_file_is_always_whole_old_or_whole_new() {
        let dir = tmp("whole");
        let path = dir.join("validator.key");
        let old = vec![b'o'; 4_096];
        let new = vec![b'n'; 4_096];
        replace(&path, &old, 0o600).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), old);

        // A reader racing the replacements sees only full contents, never a
        // mix or a truncation — the property a partial write would break.
        let reader = std::thread::spawn({
            let path = path.clone();
            let (whole_old, whole_new) = (old.clone(), new.clone());
            move || {
                for _ in 0..2_000 {
                    let got = std::fs::read(&path).expect("the file always exists");
                    assert!(got == whole_old || got == whole_new, "a partial write became visible");
                }
            }
        });
        for i in 0..200 {
            replace(&path, if i % 2 == 0 { &new } else { &old }, 0o600).unwrap();
        }
        reader.join().unwrap();

        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A write that cannot even start (the parent path is a plain file) fails
    /// without touching anything.
    #[test]
    fn a_write_that_cannot_start_fails_cleanly() {
        let dir = tmp("blocked");
        let path = dir.join("nested").join("key");
        std::fs::write(dir.join("nested"), b"not a dir").unwrap();
        assert!(replace(&path, b"new", 0o600).is_err());
        assert_eq!(std::fs::read(dir.join("nested")).unwrap(), b"not a dir");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creating_a_key_never_replaces_an_existing_identity() {
        let dir = tmp("exclusive");
        let path = dir.join("validator.key");
        create(&path, b"first identity", 0o600).unwrap();
        assert!(create(&path, b"replacement identity", 0o600).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first identity");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
