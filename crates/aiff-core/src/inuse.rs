//! Decide whether Serato may still be writing a file. Repairing a live recording would corrupt it.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// A file touched this recently is assumed to be mid-write.
pub const RECENT_WINDOW: Duration = Duration::from_secs(120);

pub fn recently_modified(modified: SystemTime, now: SystemTime, window: Duration) -> bool {
    match now.duration_since(modified) {
        Ok(age) => age < window,
        Err(_) => true, // mtime in the future: clock skew, be conservative
    }
}

/// True if another process holds the file open for writing (Windows), or it was modified within `window`.
pub fn is_in_use(path: &Path, window: Duration) -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const ERROR_SHARING_VIOLATION: i32 = 32;
        // Asking to share only reads fails exactly when someone else has a writer open,
        // while concurrent readers (our own scans, previews) are unaffected.
        match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
        {
            Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => return Ok(true),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
    }
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(recently_modified(modified, SystemTime::now(), window))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_modification_counts_as_in_use() {
        let now = SystemTime::now();
        assert!(recently_modified(
            now - Duration::from_secs(5),
            now,
            RECENT_WINDOW
        ));
        assert!(!recently_modified(
            now - Duration::from_secs(600),
            now,
            RECENT_WINDOW
        ));
    }

    #[test]
    fn a_future_mtime_counts_as_recent() {
        let now = SystemTime::now();
        assert!(recently_modified(
            now + Duration::from_secs(60),
            now,
            RECENT_WINDOW
        ));
    }

    #[test]
    fn old_idle_file_is_not_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.aif");
        std::fs::write(&p, b"x").unwrap();
        // zero window: only a held lock can make it "in use"
        assert!(!is_in_use(&p, Duration::ZERO).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn a_file_held_open_by_a_writer_is_in_use() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("live.aif");
        let _held = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(1) // FILE_SHARE_READ only: no other writer allowed
            .open(&p)
            .unwrap();
        assert!(is_in_use(&p, Duration::ZERO).unwrap());
    }
}
