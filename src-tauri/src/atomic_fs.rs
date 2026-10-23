//! Durable atomic file replace for prefs / tasks / small JSON state.
//!
//! On Windows, when the destination already exists we use `ReplaceFileW` so there
//! is no delete-then-rename empty window. New files use rename only.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Write `contents` to `path` via a unique same-directory temp file, then atomically
/// replace (or create) the destination.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "atomic write: missing parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;

    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), seq));

    let write_result = (|| -> Result<(), String> {
        {
            let mut f = File::create(&tmp).map_err(|e| e.to_string())?;
            f.write_all(contents.as_ref()).map_err(|e| e.to_string())?;
            f.flush().map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        replace_or_rename(&tmp, path)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

fn replace_or_rename(tmp: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        replace_existing(tmp, dst)
    } else {
        fs::rename(tmp, dst).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
fn replace_existing(tmp: &Path, dst: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};

    let tmp_wide: Vec<u16> = tmp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dst_wide: Vec<u16> = dst
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // lpReplacedFileName = destination; lpReplacementFileName = temp.
    // No backup; avoid delete+rename gap on existing targets.
    unsafe {
        ReplaceFileW(
            PCWSTR(dst_wide.as_ptr()),
            PCWSTR(tmp_wide.as_ptr()),
            PCWSTR::null(),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }
    .map_err(|e| format!("ReplaceFileW: {e}"))
}

#[cfg(not(windows))]
fn replace_existing(tmp: &Path, dst: &Path) -> Result<(), String> {
    // POSIX rename replaces atomically when same filesystem.
    fs::rename(tmp, dst).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xmu-atomic-fs-{}-{}-{}",
            label,
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_creates_new_file() {
        let dir = temp_dir("create");
        let path = dir.join("shell.json");
        write_atomic(&path, b"{\"a\":1}\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_replaces_existing_without_truncating_window() {
        let dir = temp_dir("replace");
        let path = dir.join("arm.json");
        write_atomic(&path, b"old-content-xxxxxxxx").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        // No leftover *.tmp in the directory after success.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "tmp leftovers: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
