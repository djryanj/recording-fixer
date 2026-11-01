//! On-disk peak cache, keyed by file path + size + modification time, with size-capped eviction.

use crate::{Error, Peaks, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAGIC: &[u8; 4] = b"SRFP";
const VERSION: u32 = 1;

/// Identifies one version of one file; any change to it invalidates the cache entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub path: String,
    pub size: u64,
    pub modified_ms: u128,
}

impl Key {
    pub fn for_file(path: &Path) -> std::io::Result<Key> {
        let meta = fs::metadata(path)?;
        let modified_ms = meta
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        Ok(Key {
            path: path.to_string_lossy().into_owned(),
            size: meta.len(),
            modified_ms,
        })
    }

    fn file_name(&self) -> String {
        // FNV-1a: stable across runs and platforms (unlike the std hasher).
        let mut h = 0xcbf29ce484222325u64;
        let text = format!(
            "{}|{}|{}|v{VERSION}",
            self.path, self.size, self.modified_ms
        );
        for b in text.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}.peaks")
    }
}

pub fn path_for(dir: &Path, key: &Key) -> PathBuf {
    dir.join(key.file_name())
}

pub fn save(dir: &Path, key: &Key, peaks: &Peaks) -> Result<()> {
    fs::create_dir_all(dir)?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&peaks.frames.to_le_bytes());
    out.extend_from_slice(&(peaks.levels.len() as u32).to_le_bytes());
    for level in &peaks.levels {
        out.extend_from_slice(&(level.len() as u64).to_le_bytes());
        out.extend(level.iter().map(|&v| v as u8));
    }
    // Write then rename so a crash never leaves a half-written cache file.
    let final_path = path_for(dir, key);
    let tmp = final_path.with_extension("tmp");
    fs::File::create(&tmp)?.write_all(&out)?;
    fs::rename(tmp, final_path)?;
    Ok(())
}

pub fn load(dir: &Path, key: &Key) -> Result<Option<Peaks>> {
    let path = path_for(dir, key);
    let mut bytes = Vec::new();
    match fs::File::open(&path) {
        Ok(mut f) => f.read_to_end(&mut bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    match parse(&bytes) {
        Ok(p) => Ok(Some(p)),
        Err(_) => {
            // Unreadable cache entries are just discarded and rebuilt.
            let _ = fs::remove_file(&path);
            Ok(None)
        }
    }
}

fn parse(b: &[u8]) -> Result<Peaks> {
    let take = |pos: &mut usize, n: usize| -> Result<&[u8]> {
        let s = b.get(*pos..*pos + n).ok_or(Error::Corrupt)?;
        *pos += n;
        Ok(s)
    };
    let mut pos = 0;
    if take(&mut pos, 4)? != MAGIC
        || u32::from_le_bytes(take(&mut pos, 4)?.try_into().unwrap()) != VERSION
    {
        return Err(Error::Corrupt);
    }
    let frames = u64::from_le_bytes(take(&mut pos, 8)?.try_into().unwrap());
    let count = u32::from_le_bytes(take(&mut pos, 4)?.try_into().unwrap()) as usize;
    let mut levels = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let len = u64::from_le_bytes(take(&mut pos, 8)?.try_into().unwrap()) as usize;
        levels.push(take(&mut pos, len)?.iter().map(|&v| v as i8).collect());
    }
    if pos != b.len() || levels.is_empty() {
        return Err(Error::Corrupt);
    }
    Ok(Peaks { frames, levels })
}

/// Delete least-recently-modified cache files until the folder is at most `max_bytes`.
pub fn prune(dir: &Path, max_bytes: u64) -> std::io::Result<()> {
    prune_ext(dir, "peaks", max_bytes)
}

/// [`prune`] for cache files with another extension (other kinds of cache share this scheme).
pub fn prune_ext(dir: &Path, ext: &str, max_bytes: u64) -> std::io::Result<()> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(());
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, size, path) in files {
        if total <= max_bytes {
            break;
        }
        fs::remove_file(path)?;
        total -= size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Peaks {
        Peaks {
            frames: 123_456,
            levels: vec![vec![-5, 5, -100, 100, 0, 1], vec![-100, 100]],
        }
    }

    fn key(path: &str, size: u64, ms: u128) -> Key {
        Key {
            path: path.into(),
            size,
            modified_ms: ms,
        }
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let k = key("a.aif", 10, 20);
        assert_eq!(load(dir.path(), &k).unwrap(), None);
        save(dir.path(), &k, &sample()).unwrap();
        assert_eq!(load(dir.path(), &k).unwrap(), Some(sample()));
    }

    #[test]
    fn changing_the_file_invalidates_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &key("a.aif", 10, 20), &sample()).unwrap();
        assert_eq!(load(dir.path(), &key("a.aif", 11, 20)).unwrap(), None);
        assert_eq!(load(dir.path(), &key("a.aif", 10, 21)).unwrap(), None);
        assert_eq!(load(dir.path(), &key("b.aif", 10, 20)).unwrap(), None);
    }

    #[test]
    fn corrupt_entries_are_discarded_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let k = key("a.aif", 1, 1);
        for junk in [
            &b""[..],
            b"SRFP",
            b"XXXXjunkjunkjunkjunkjunk",
            b"SRFP\x01\x00\x00\x00\x00",
        ] {
            fs::write(path_for(dir.path(), &k), junk).unwrap();
            assert_eq!(load(dir.path(), &k).unwrap(), None);
            assert!(!path_for(dir.path(), &k).exists());
        }
    }

    #[test]
    fn truncated_entries_are_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let k = key("a.aif", 1, 1);
        save(dir.path(), &k, &sample()).unwrap();
        let mut bytes = fs::read(path_for(dir.path(), &k)).unwrap();
        bytes.truncate(bytes.len() - 1);
        fs::write(path_for(dir.path(), &k), bytes).unwrap();
        assert_eq!(load(dir.path(), &k).unwrap(), None);
    }

    #[test]
    fn prune_removes_the_oldest_until_under_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..4u8 {
            let p = dir.path().join(format!("{i}.peaks"));
            fs::write(&p, vec![i; 100]).unwrap();
            let t = SystemTime::now() - std::time::Duration::from_secs(1000 - u64::from(i) * 100);
            fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        prune(dir.path(), 250).unwrap();
        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["2.peaks", "3.peaks"]);
    }

    #[test]
    fn prune_ignores_unrelated_files_and_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), vec![0; 1000]).unwrap();
        prune(dir.path(), 0).unwrap();
        assert!(dir.path().join("notes.txt").exists());
        prune(&dir.path().join("missing"), 0).unwrap();
    }

    #[test]
    fn key_for_file_tracks_size_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.aif");
        fs::write(&p, b"abc").unwrap();
        let a = Key::for_file(&p).unwrap();
        fs::write(&p, b"abcd").unwrap();
        assert_ne!(a, Key::for_file(&p).unwrap());
    }
}
