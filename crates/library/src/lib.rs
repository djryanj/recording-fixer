//! Finding Serato recording folders and listing the recordings in them.
//!
//! Scanning reads only each file's header and ID3 trailer, never the audio.

use aiff_core::inuse::{is_in_use, RECENT_WINDOW};
use aiff_core::{diagnose_path, Defect, Diagnosis, Error, Health};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderKind {
    Recording,
    Temp,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderInfo {
    pub path: String,
    pub label: String,
    pub kind: FolderKind,
    pub exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Broken,
    /// Another process has it open for writing, or an unclosed file was touched moments ago.
    InUse,
    Unreadable,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub folder_kind: FolderKind,
    pub size: u64,
    pub modified_ms: i64,
    pub status: Status,
    /// Human-readable explanation of the status.
    pub detail: Option<String>,
    pub duration_secs: Option<f64>,
    pub sample_rate: Option<f64>,
    pub bits: Option<u16>,
    pub channels: Option<u16>,
    pub title: Option<String>,
    /// "Recorded ..." time from Serato's ID3 tag, only present on cleanly closed files.
    pub recorded: Option<String>,
}

fn kind_of(folder: &Path) -> FolderKind {
    match folder
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("recording") => FolderKind::Recording,
        Some("recording temp") => FolderKind::Temp,
        _ => FolderKind::Other,
    }
}

/// `<music>/_Serato_/Recording` and `.../Recording temp`.
pub fn default_folders_in(music: &Path) -> Vec<FolderInfo> {
    ["Recording", "Recording temp"]
        .into_iter()
        .map(|name| {
            let path = music.join("_Serato_").join(name);
            FolderInfo {
                label: name.to_string(),
                kind: kind_of(&path),
                exists: path.is_dir(),
                path: path.display().to_string(),
            }
        })
        .collect()
}

/// Environment variable that replaces the default folders with a path list (`;` on Windows, `:`
/// elsewhere). Used by automated tests so they never touch a real Serato library.
pub const FOLDERS_ENV: &str = "RECORDING_FIXER_FOLDERS";

/// Folders from an override value if there is one, otherwise the Serato folders under `music`.
pub fn default_folders_from(
    override_paths: Option<&std::ffi::OsStr>,
    music: Option<&Path>,
) -> Vec<FolderInfo> {
    if let Some(value) = override_paths.filter(|v| !v.is_empty()) {
        return std::env::split_paths(value)
            .map(|path| FolderInfo {
                label: path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
                kind: kind_of(&path),
                exists: path.is_dir(),
                path: path.display().to_string(),
            })
            .collect();
    }
    music.map(default_folders_in).unwrap_or_default()
}

pub fn default_folders() -> Vec<FolderInfo> {
    default_folders_from(
        std::env::var_os(FOLDERS_ENV).as_deref(),
        dirs::audio_dir().as_deref(),
    )
}

pub fn describe_defects(defects: &[Defect]) -> String {
    let mut parts = Vec::new();
    for d in defects {
        parts.push(match d {
            Defect::FormSize { .. } => "file size field is wrong".to_string(),
            Defect::FrameCount { found: 0, .. } => "length is missing".to_string(),
            Defect::FrameCount { .. } => "length field is wrong".to_string(),
            Defect::SsndSize { .. } => "audio size field is wrong".to_string(),
            Defect::PartialFrame { bytes } => format!("{bytes} stray byte(s) at the end"),
        });
    }
    parts.join("; ")
}

fn entry_for(path: &Path, folder: &Path, window: Duration) -> Option<FileEntry> {
    let meta = std::fs::metadata(path).ok()?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    let mut e = FileEntry {
        path: path.display().to_string(),
        name: path.file_name()?.to_string_lossy().into_owned(),
        folder: folder.display().to_string(),
        folder_kind: kind_of(folder),
        size: meta.len(),
        modified_ms,
        status: Status::Unreadable,
        detail: None,
        duration_secs: None,
        sample_rate: None,
        bits: None,
        channels: None,
        title: None,
        recorded: None,
    };
    match diagnose_path(path) {
        Ok(d) => {
            fill_from(&mut e, &d);
            // A cleanly closed file only counts as busy while a writer holds it. An unclosed
            // one is busy until it has been idle for `window`: Serato may still be recording.
            let idle_window = if d.is_healthy() {
                Duration::ZERO
            } else {
                window
            };
            let busy = is_in_use(path, idle_window).unwrap_or(false);
            match (&d.health, busy) {
                (_, true) => {
                    e.status = Status::InUse;
                    e.detail = Some("Serato may still be recording this file".into());
                }
                (Health::Healthy, _) => e.status = Status::Ok,
                (Health::Broken(defects), _) => {
                    e.status = Status::Broken;
                    e.detail = Some(describe_defects(defects));
                }
            }
        }
        Err(Error::Unsupported(what)) => {
            e.status = Status::Unsupported;
            e.detail = Some(format!("{what} is not supported"));
        }
        Err(err) => e.detail = Some(err.to_string()),
    }
    Some(e)
}

fn fill_from(e: &mut FileEntry, d: &Diagnosis) {
    e.duration_secs = Some(d.duration_secs());
    e.sample_rate = Some(d.format.sample_rate);
    e.bits = Some(d.format.bits);
    e.channels = Some(d.format.channels);
    if let Some(t) = &d.tags {
        e.title = t.title.clone();
        e.recorded = t.recorded.clone();
    }
}

fn is_aiff_name(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("aif") || e.eq_ignore_ascii_case("aiff"))
}

/// List every `.aif`/`.aiff` directly inside the given folders (not recursive).
/// Missing folders are skipped. Newest first.
pub fn scan(folders: &[PathBuf]) -> Vec<FileEntry> {
    scan_with(folders, RECENT_WINDOW)
}

pub fn scan_with(folders: &[PathBuf], window: Duration) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for folder in folders {
        let Ok(rd) = std::fs::read_dir(folder) else {
            continue;
        };
        for item in rd.flatten() {
            let p = item.path();
            if p.is_file() && is_aiff_name(&p) {
                out.extend(entry_for(&p, folder, window));
            }
        }
    }
    out.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// `dir/<stem><suffix>.<ext>`, or with " 2", " 3"... if that name is taken.
pub fn unique_path(dir: &Path, stem: &str, suffix: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}{suffix}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem}{suffix} {n}.{ext}")))
        .find(|p| !p.exists())
        .expect("infinite range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiff_core::fixtures::{self, Spec};
    use std::fs;

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn default_folders_follow_serato_layout() {
        let music = tempfile::tempdir().unwrap();
        fs::create_dir_all(music.path().join("_Serato_").join("Recording")).unwrap();
        let f = default_folders_in(music.path());
        assert_eq!(f.len(), 2);
        assert_eq!(
            (f[0].label.as_str(), f[0].kind.clone(), f[0].exists),
            ("Recording", FolderKind::Recording, true)
        );
        assert_eq!(
            (f[1].label.as_str(), f[1].kind.clone(), f[1].exists),
            ("Recording temp", FolderKind::Temp, false)
        );
        assert!(f[1].path.ends_with("Recording temp"));
    }

    #[test]
    fn an_override_replaces_the_serato_folders() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let joined =
            std::env::join_paths([a.path(), b.path(), Path::new("/definitely/missing")]).unwrap();
        let music = tempfile::tempdir().unwrap();
        let f = default_folders_from(Some(&joined), Some(music.path()));
        assert_eq!(f.len(), 3);
        assert!(f[0].exists && f[1].exists && !f[2].exists);
        assert!(f.iter().all(|x| !x.path.contains("_Serato_")));
    }

    #[test]
    fn an_empty_or_absent_override_uses_the_music_folder() {
        let music = tempfile::tempdir().unwrap();
        for ov in [None, Some(std::ffi::OsStr::new(""))] {
            let f = default_folders_from(ov, Some(music.path()));
            assert_eq!(f.len(), 2);
            assert!(f[0].path.contains("_Serato_"));
        }
        assert!(default_folders_from(None, None).is_empty());
    }

    #[test]
    fn classifies_good_broken_and_garbage_files() {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "good.aif",
            &fixtures::healthy(Spec::cd(44100), 1),
        );
        put(
            dir.path(),
            "bad.aif",
            &fixtures::broken(Spec::cd(44100), 1, 0),
        );
        put(dir.path(), "junk.aif", b"this is not audio");
        put(dir.path(), "notes.txt", b"ignored");
        let got = scan_with(&[dir.path().to_path_buf()], Duration::ZERO);
        let status = |n: &str| got.iter().find(|e| e.name == n).unwrap().status;
        assert_eq!(got.len(), 3);
        assert_eq!(status("good.aif"), Status::Ok);
        assert_eq!(status("bad.aif"), Status::Broken);
        assert_eq!(status("junk.aif"), Status::Unreadable);
    }

    #[test]
    fn entries_carry_format_duration_and_tags() {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "good.aif",
            &fixtures::healthy(Spec::cd(88200), 1),
        );
        let e = &scan_with(&[dir.path().to_path_buf()], Duration::ZERO)[0];
        assert_eq!(e.duration_secs, Some(2.0));
        assert_eq!(
            (e.sample_rate, e.bits, e.channels),
            (Some(44100.0), Some(16), Some(2))
        );
        assert_eq!(e.recorded.as_deref(), Some("2025-10-04 10:31:51"));
        assert_eq!(e.title.as_deref(), Some("Serato Recording"));
    }

    #[test]
    fn broken_entries_explain_what_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "bad.aif",
            &fixtures::broken(Spec::cd(100), 1, 3),
        );
        let e = &scan_with(&[dir.path().to_path_buf()], Duration::ZERO)[0];
        assert!(e.detail.as_deref().unwrap().contains("length is missing"));
        assert!(e.detail.as_deref().unwrap().contains("stray"));
    }

    #[test]
    fn a_just_written_unclosed_file_is_in_use_but_a_just_written_good_file_is_not() {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "live.aif",
            &fixtures::broken(Spec::cd(100), 1, 0),
        );
        put(dir.path(), "done.aif", &fixtures::healthy(Spec::cd(100), 1));
        let got = scan_with(&[dir.path().to_path_buf()], Duration::from_secs(3600));
        let status = |n: &str| got.iter().find(|e| e.name == n).unwrap().status;
        assert_eq!(status("live.aif"), Status::InUse);
        assert_eq!(status("done.aif"), Status::Ok);
    }

    #[test]
    fn aifc_is_reported_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = fixtures::healthy(Spec::cd(10), 1);
        b[8..12].copy_from_slice(b"AIFC");
        put(dir.path(), "c.aif", &b);
        assert_eq!(
            scan_with(&[dir.path().to_path_buf()], Duration::ZERO)[0].status,
            Status::Unsupported
        );
    }

    #[test]
    fn missing_folders_are_skipped_and_both_extensions_match() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "a.AIF", &fixtures::healthy(Spec::cd(10), 1));
        put(dir.path(), "b.aiff", &fixtures::healthy(Spec::cd(10), 2));
        let got = scan_with(
            &[dir.path().join("nope"), dir.path().to_path_buf()],
            Duration::ZERO,
        );
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn folder_kind_is_derived_from_the_directory_name() {
        let music = tempfile::tempdir().unwrap();
        let temp = music.path().join("_Serato_").join("Recording temp");
        fs::create_dir_all(&temp).unwrap();
        put(&temp, "x.aif", &fixtures::broken(Spec::cd(10), 1, 0));
        assert_eq!(
            scan_with(&[temp], Duration::ZERO)[0].folder_kind,
            FolderKind::Temp
        );
    }

    #[test]
    fn unique_path_avoids_collisions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(unique_path(dir.path(), "a", " (fixed)", "aif").ends_with("a (fixed).aif"));
        put(dir.path(), "a (fixed).aif", b"");
        assert!(unique_path(dir.path(), "a", " (fixed)", "aif").ends_with("a (fixed) 2.aif"));
        put(dir.path(), "a (fixed) 2.aif", b"");
        assert!(unique_path(dir.path(), "a", " (fixed)", "aif").ends_with("a (fixed) 3.aif"));
    }
}
