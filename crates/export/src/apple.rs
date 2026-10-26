//! Detect Apple's AAC encoder, which sounds better than ffmpeg's native one.
//!
//! * macOS: AudioToolbox is part of the OS; ffmpeg exposes it as `aac_at` when built with it.
//! * Windows: ffmpeg cannot use Apple's encoder. OBS loads `CoreAudioToolbox.dll`, which only
//!   exists when Apple Application Support (installed with iTunes) is present. We load it the
//!   same way (see `apple_aac`) and never bundle it.

use crate::Encoders;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum AppleAac {
    /// The ffmpeg in use has `aac_at`.
    FfmpegAudioToolbox,
    /// macOS system tool (detected, not used).
    Afconvert(PathBuf),
    /// Windows: Apple's DLL is installed and loads. `usable` is false if probing failed.
    CoreAudioToolboxDll { path: PathBuf, usable: bool },
}

/// Where Apple Application Support / iTunes put `CoreAudioToolbox.dll`.
pub fn windows_dll_candidates(program_files: &[PathBuf], common_files: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in common_files {
        out.push(
            dir.join("Apple")
                .join("Apple Application Support")
                .join("CoreAudioToolbox.dll"),
        );
    }
    for dir in program_files {
        out.push(dir.join("iTunes").join("CoreAudioToolbox.dll"));
        out.push(
            dir.join("Common Files")
                .join("Apple")
                .join("Apple Application Support")
                .join("CoreAudioToolbox.dll"),
        );
    }
    out
}

fn env_paths(names: &[&str]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = names
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    v.dedup();
    v
}

/// First existing `CoreAudioToolbox.dll` (64-bit locations first). Always `None` off Windows.
pub fn find_windows_dll() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    // A 64-bit process sees the 64-bit Program Files in `ProgramFiles`; avoid the x86 copy,
    // which is a 32-bit DLL we could not load.
    let program_files = env_paths(&["ProgramW6432", "ProgramFiles"]);
    let common = env_paths(&["CommonProgramW6432", "CommonProgramFiles"]);
    windows_dll_candidates(&program_files, &common)
        .into_iter()
        .find(|p| p.is_file())
}

pub fn detect(encoders: &Encoders) -> Option<AppleAac> {
    if encoders.has("aac_at") {
        return Some(AppleAac::FfmpegAudioToolbox);
    }
    let afconvert = Path::new("/usr/bin/afconvert");
    if cfg!(target_os = "macos") && afconvert.is_file() {
        return Some(AppleAac::Afconvert(afconvert.to_path_buf()));
    }
    let path = find_windows_dll()?;
    #[cfg(windows)]
    let usable = crate::apple_aac::probe(&path).is_ok();
    #[cfg(not(windows))]
    let usable = false;
    Some(AppleAac::CoreAudioToolboxDll { path, usable })
}

/// The DLL path if Apple's encoder can actually be used here.
pub fn usable_windows_dll() -> Option<PathBuf> {
    match detect(&Encoders::default())? {
        AppleAac::CoreAudioToolboxDll { path, usable: true } => Some(path),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_aac_at_is_detected_first() {
        let e = Encoders {
            names: vec!["aac".into(), "aac_at".into()],
        };
        assert_eq!(detect(&e), Some(AppleAac::FfmpegAudioToolbox));
    }

    #[test]
    fn dll_candidates_cover_the_itunes_and_app_support_locations() {
        let c = windows_dll_candidates(
            &[PathBuf::from("C:/Program Files")],
            &[PathBuf::from("C:/Program Files/Common Files")],
        );
        let s: Vec<String> = c
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.contains(
            &"C:/Program Files/Common Files/Apple/Apple Application Support/CoreAudioToolbox.dll"
                .to_string()
        ));
        assert!(s.contains(&"C:/Program Files/iTunes/CoreAudioToolbox.dll".to_string()));
    }
}
