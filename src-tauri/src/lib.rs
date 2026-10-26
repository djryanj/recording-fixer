//! Thin Tauri command layer. All real work lives in the crates under `crates/`.

use aiff_core::inuse::{is_in_use, RECENT_WINDOW};
use edit_core::Clip;
use export::{AacMode, Codec, ExportSpec, Metadata, Mp3Mode, Mp3Options, APPLE_ENCODER};
use library::{FileEntry, FolderInfo};
use peaks::{cache, Peaks};
use playback::{DeviceInfo, DeviceSelection, LoadInfo, OpenedInfo, Playback, PlaybackState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared cancel flag for whichever job is running (one at a time).
#[derive(Default)]
struct Jobs {
    cancel: Arc<AtomicBool>,
}

/// Waveform peaks: a few recently used files in memory, everything else on disk.
#[derive(Default)]
struct PeaksStore {
    recent: Mutex<Vec<(String, Arc<Peaks>)>>,
    /// Serialises builds so several clips of one file never analyse it twice.
    build: Mutex<()>,
}

const PEAKS_IN_MEMORY: usize = 8;
const PEAK_CACHE_MAX_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Serialize, Clone)]
struct ProgressEvent {
    label: String,
    done: u64,
    total: u64,
}

/// Emits `job-progress` at most ~10 times a second, and always for the final update.
struct Throttle {
    app: AppHandle,
    label: String,
    last: Instant,
}

impl Throttle {
    fn new(app: &AppHandle, label: impl Into<String>) -> Self {
        Throttle {
            app: app.clone(),
            label: label.into(),
            last: Instant::now() - Duration::from_secs(1),
        }
    }
    fn emit(&mut self, done: u64, total: u64) {
        if done >= total || self.last.elapsed() >= Duration::from_millis(100) {
            self.last = Instant::now();
            let _ = self.app.emit(
                "job-progress",
                ProgressEvent {
                    label: self.label.clone(),
                    done,
                    total,
                },
            );
        }
    }
}

#[tauri::command]
fn default_folders() -> Vec<FolderInfo> {
    library::default_folders()
}

#[tauri::command]
async fn scan(folders: Vec<String>) -> Result<Vec<FileEntry>, String> {
    let folders: Vec<PathBuf> = folders.into_iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || library::scan(&folders))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct RepairResult {
    path: String,
    ok: bool,
    output: Option<String>,
    error: Option<String>,
}

fn repair_one(
    app: &AppHandle,
    cancel: &AtomicBool,
    path: &str,
    in_place: bool,
) -> Result<String, String> {
    let src = Path::new(path);
    if is_in_use(src, RECENT_WINDOW).map_err(|e| e.to_string())? {
        return Err("Serato may still be recording this file; close Serato and try again".into());
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut throttle = Throttle::new(app, format!("Repairing {name}"));
    if in_place {
        aiff_core::repair_in_place(src, true).map_err(|e| e.to_string())?;
        return Ok(path.to_string());
    }
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dst = library::unique_path(
        src.parent().unwrap_or(Path::new(".")),
        &stem,
        " (fixed)",
        "aif",
    );
    aiff_core::repair_copy(src, &dst, &mut |done, total| {
        throttle.emit(done, total);
        !cancel.load(Ordering::Relaxed)
    })
    .map_err(|e| e.to_string())?;
    Ok(dst.display().to_string())
}

#[tauri::command]
async fn repair(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    paths: Vec<String>,
    in_place: bool,
) -> Result<Vec<RepairResult>, String> {
    let cancel = jobs.cancel.clone();
    cancel.store(false, Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| match repair_one(&app, &cancel, &path, in_place) {
                Ok(output) => RepairResult {
                    path,
                    ok: true,
                    output: Some(output),
                    error: None,
                },
                Err(error) => RepairResult {
                    path,
                    ok: false,
                    output: None,
                    error: Some(error),
                },
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct ToolStatus {
    ffmpeg_path: Option<String>,
    ffmpeg_version: Option<String>,
    /// Selectable AAC encoders, best first. May include the Apple pseudo-encoder.
    aac_encoders: Vec<String>,
    best_aac: Option<String>,
    mp3: bool,
    /// How Apple's AAC encoder is (or is not) available, for display.
    apple: Option<String>,
}

fn bundled_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
            dirs.push(d.join("binaries"));
        }
    }
    if let Ok(r) = app.path().resource_dir() {
        dirs.push(r);
    }
    dirs
}

fn find_ffmpeg(app: &AppHandle, user: Option<&str>) -> Option<PathBuf> {
    let user = user.filter(|s| !s.is_empty()).map(PathBuf::from);
    export::locate_ffmpeg(user.as_deref(), &bundled_dirs(app))
}

fn describe_apple(a: &export::apple::AppleAac) -> String {
    use export::apple::AppleAac::*;
    match a {
        FfmpegAudioToolbox => "Apple AAC via ffmpeg (aac_at)".into(),
        Afconvert(_) => "Apple AAC found (afconvert); not used".into(),
        CoreAudioToolboxDll { usable: true, .. } => "Apple AAC (CoreAudioToolbox)".into(),
        CoreAudioToolboxDll { usable: false, .. } => {
            "Apple CoreAudioToolbox found but could not be loaded".into()
        }
    }
}

#[tauri::command]
async fn tool_status(app: AppHandle, user_path: Option<String>) -> Result<ToolStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = find_ffmpeg(&app, user_path.as_deref()) else {
            let apple =
                export::apple::detect(&export::Encoders::default()).map(|a| describe_apple(&a));
            return ToolStatus {
                ffmpeg_path: None,
                ffmpeg_version: None,
                aac_encoders: vec![],
                best_aac: None,
                mp3: false,
                apple,
            };
        };
        let encoders = export::probe_encoders(&path).unwrap_or_default();
        let apple = export::apple::detect(&encoders);
        let mut aac: Vec<String> = encoders
            .aac_available()
            .into_iter()
            .map(String::from)
            .collect();
        // Apple's encoder driven directly (Windows) is the best choice when it works.
        if export::apple::usable_windows_dll().is_some() {
            aac.insert(0, APPLE_ENCODER.to_string());
        }
        ToolStatus {
            ffmpeg_version: export::version(&path),
            ffmpeg_path: Some(path.display().to_string()),
            best_aac: aac.first().cloned(),
            aac_encoders: aac,
            mp3: encoders.has_mp3(),
            apple: apple.as_ref().map(describe_apple),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Deserialize, Clone)]
struct ClipDto {
    path: String,
    start_frame: u64,
    end_frame: Option<u64>,
}

fn to_clips(dtos: &[ClipDto]) -> Vec<Clip> {
    dtos.iter()
        .map(|c| Clip {
            path: PathBuf::from(&c.path),
            start_frame: c.start_frame,
            end_frame: c.end_frame,
        })
        .collect()
}

#[derive(Serialize)]
struct PlanInfo {
    total_frames: u64,
    duration_secs: f64,
    bytes: u64,
    format: String,
    fits_aiff: bool,
}

#[tauri::command]
async fn plan_info(clips: Vec<ClipDto>) -> Result<PlanInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let plan = edit_core::plan(&to_clips(&clips)).map_err(|e| e.to_string())?;
        Ok(PlanInfo {
            total_frames: plan.total_frames,
            duration_secs: plan.duration_secs(),
            bytes: plan.total_bytes(),
            format: format!(
                "{} Hz, {}-bit, {} ch",
                plan.format.sample_rate, plan.format.bits, plan.format.channels
            ),
            fits_aiff: edit_core::fits_in_aiff(plan.total_bytes()),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Target {
    Aiff,
    /// `mode` is "cbr", "vbr" (value = LAME V 0-9) or "abr".
    Mp3 {
        mode: String,
        value: u32,
        id3_version: u8,
        id3v1: bool,
    },
    /// `mode` is "cbr" (value = kbps) or "vbr" (value = quality level 1-5).
    Aac {
        encoder: String,
        mode: String,
        value: u32,
    },
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MetadataDto {
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    genre: String,
    date: String,
    comment: String,
    track: String,
}

impl From<MetadataDto> for Metadata {
    fn from(m: MetadataDto) -> Self {
        Metadata {
            title: m.title,
            artist: m.artist,
            album: m.album,
            album_artist: m.album_artist,
            genre: m.genre,
            date: m.date,
            comment: m.comment,
            track: m.track,
        }
    }
}

#[derive(Deserialize)]
struct ExportJob {
    clips: Vec<ClipDto>,
    target: Target,
    output_dir: String,
    /// Final file name without extension; the UI has already expanded any template.
    file_stem: String,
    #[serde(default)]
    metadata: MetadataDto,
    ffmpeg_path: Option<String>,
    /// Fade in over the first N seconds of the result, and out over the last N (0 = none).
    #[serde(default)]
    fade_in_secs: f64,
    #[serde(default)]
    fade_out_secs: f64,
}

/// Keep a user-typed file name from escaping the output folder or being invalid on Windows.
fn sanitize_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() {
        "Serato Recording".into()
    } else {
        cleaned
    }
}

fn codec_for(target: Target) -> Result<Codec, String> {
    Ok(match target {
        Target::Aiff => return Err("AIFF has no codec".into()),
        Target::Mp3 {
            mode,
            value,
            id3_version,
            id3v1,
        } => {
            let mode = match mode.as_str() {
                "vbr" => Mp3Mode::Vbr(value.min(9) as u8),
                "abr" => Mp3Mode::Abr(value),
                _ => Mp3Mode::Cbr(value),
            };
            Codec::Mp3(Mp3Options {
                mode,
                id3_version: if id3_version == 4 { 4 } else { 3 },
                id3v1,
            })
        }
        Target::Aac {
            encoder,
            mode,
            value,
        } => {
            let mode = if mode == "vbr" {
                AacMode::Vbr(value.clamp(1, 5) as u8)
            } else {
                AacMode::Cbr(value)
            };
            Codec::Aac { encoder, mode }
        }
    })
}

fn run_export(app: &AppHandle, cancel: &AtomicBool, job: ExportJob) -> Result<String, String> {
    let plan = edit_core::plan(&to_clips(&job.clips)).map_err(|e| e.to_string())?;
    let stem = sanitize_stem(&job.file_stem);
    let ext = match &job.target {
        Target::Aiff => "aif",
        Target::Mp3 { .. } => "mp3",
        Target::Aac { .. } => "m4a",
    };
    let out = library::unique_path(Path::new(&job.output_dir), &stem, "", ext);
    let mut throttle = Throttle::new(app, format!("Exporting {stem}.{ext}"));
    let mut progress = |done: u64, total: u64| {
        throttle.emit(done, total);
        !cancel.load(Ordering::Relaxed)
    };

    let fade =
        edit_core::Fade::from_secs(job.fade_in_secs, job.fade_out_secs, plan.format.sample_rate);
    if matches!(job.target, Target::Aiff) {
        plan.write_aiff_faded(&out, fade, &mut progress)
            .map_err(|e| e.to_string())?;
        return Ok(out.display().to_string());
    }
    let codec = codec_for(job.target)?;
    let ffmpeg = find_ffmpeg(app, job.ffmpeg_path.as_deref())
        .ok_or_else(|| export::Error::FfmpegNotFound.to_string())?;
    let needed = match &codec {
        Codec::Mp3(_) => Some("libmp3lame"),
        Codec::Aac { encoder, .. } if encoder == APPLE_ENCODER => None, // checked by the encoder itself
        Codec::Aac { encoder, .. } => Some(encoder.as_str()),
    };
    if let Some(name) = needed {
        let encoders = export::probe_encoders(&ffmpeg).map_err(|e| e.to_string())?;
        if !encoders.has(name) {
            return Err(export::Error::MissingEncoder(name.into()).to_string());
        }
    }
    let spec = ExportSpec {
        format: plan.format.clone(),
        codec,
        output: out.clone(),
        metadata: job.metadata.into(),
    };
    export::run(
        &ffmpeg,
        &spec,
        &mut plan.reader_faded(fade),
        plan.total_bytes(),
        &mut progress,
    )
    .map_err(|e| e.to_string())?;
    Ok(out.display().to_string())
}

#[tauri::command]
async fn export_audio(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job: ExportJob,
) -> Result<String, String> {
    let cancel = jobs.cancel.clone();
    cancel.store(false, Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || run_export(&app, &cancel, job))
        .await
        .map_err(|e| e.to_string())?
}

/// Peaks for `path`: from memory, else the disk cache, else built now (one pass over the audio).
fn peaks_for(app: &AppHandle, store: &PeaksStore, path: &str) -> Result<Arc<Peaks>, String> {
    let file = Path::new(path);
    let key = cache::Key::for_file(file).map_err(|e| e.to_string())?;
    let mem_key = format!("{}|{}|{}", key.path, key.size, key.modified_ms);
    let lookup = |store: &PeaksStore| {
        store.recent.lock().ok().and_then(|r| {
            r.iter()
                .find(|(k, _)| *k == mem_key)
                .map(|(_, p)| p.clone())
        })
    };
    if let Some(p) = lookup(store) {
        return Ok(p);
    }
    let _building = store.build.lock().map_err(|e| e.to_string())?;
    if let Some(p) = lookup(store) {
        return Ok(p); // another request finished it while we waited
    }
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("peaks");
    let peaks = match cache::load(&dir, &key).map_err(|e| e.to_string())? {
        Some(p) => p,
        None => {
            let built = peaks::build_for_file(file, &mut |_, _| true).map_err(|e| e.to_string())?;
            if cache::save(&dir, &key, &built).is_ok() {
                let _ = cache::prune(&dir, PEAK_CACHE_MAX_BYTES);
            }
            built
        }
    };
    let peaks = Arc::new(peaks);
    if let Ok(mut recent) = store.recent.lock() {
        recent.push((mem_key, peaks.clone()));
        if recent.len() > PEAKS_IN_MEMORY {
            recent.remove(0);
        }
    }
    Ok(peaks)
}

#[derive(Serialize)]
struct PeakInfo {
    frames: u64,
}

/// Make sure peaks exist for a file (building them if needed) and report its length in frames.
#[tauri::command]
async fn ensure_peaks(
    app: AppHandle,
    store: State<'_, Arc<PeaksStore>>,
    path: String,
) -> Result<PeakInfo, String> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        peaks_for(&app, &store, &path).map(|p| PeakInfo { frames: p.frames })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `width` columns of interleaved `[min, max]` signed bytes for frames `[start, end)`, as raw binary.
#[tauri::command]
async fn get_peaks(
    app: AppHandle,
    store: State<'_, Arc<PeaksStore>>,
    path: String,
    start: u64,
    end: u64,
    width: usize,
) -> Result<tauri::ipc::Response, String> {
    let store = store.inner().clone();
    let width = width.min(8192);
    tauri::async_runtime::spawn_blocking(move || {
        let peaks = peaks_for(&app, &store, &path)?;
        let bytes: Vec<u8> = peaks
            .columns(start, end, width)
            .into_iter()
            .map(|v| v as u8)
            .collect();
        Ok::<_, String>(tauri::ipc::Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Exact 16-bit `[min, max]` per frame for frames `[start, end)` (little-endian i16 pairs), for
/// zooming in to individual samples. At most `peaks::MAX_SAMPLE_FRAMES` (262,144) frames per call.
#[tauri::command]
async fn get_samples(path: String, start: u64, end: u64) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let v = peaks::read_samples(Path::new(&path), start, end).map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = v.into_iter().flat_map(i16::to_le_bytes).collect();
        Ok::<_, String>(tauri::ipc::Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_job(jobs: State<'_, Jobs>) {
    jobs.cancel.store(true, Ordering::Relaxed);
}

// ---- audio playback -------------------------------------------------------------------
// Commands that talk to the engine thread and may take a while (opening a device, loading a
// plan) run on the blocking pool; the cheap ones are plain synchronous commands.

#[tauri::command]
async fn audio_devices(pb: State<'_, Arc<Playback>>) -> Result<Vec<DeviceInfo>, String> {
    let pb = pb.inner().clone();
    tauri::async_runtime::spawn_blocking(move || pb.devices())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn audio_device_details(
    pb: State<'_, Arc<Playback>>,
    id: String,
) -> Result<Option<DeviceInfo>, String> {
    let pb = pb.inner().clone();
    tauri::async_runtime::spawn_blocking(move || pb.device_details(&id))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn audio_set_device(
    pb: State<'_, Arc<Playback>>,
    selection: DeviceSelection,
) -> Result<OpenedInfo, String> {
    let pb = pb.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        pb.set_device(selection).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn audio_load(
    pb: State<'_, Arc<Playback>>,
    clips: Vec<ClipDto>,
    start_secs: f64,
) -> Result<LoadInfo, String> {
    let pb = pb.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        pb.load(to_clips(&clips), start_secs)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn audio_play(pb: State<'_, Arc<Playback>>) {
    pb.play();
}

#[tauri::command]
fn audio_pause(pb: State<'_, Arc<Playback>>) {
    pb.pause();
}

#[tauri::command]
fn audio_seek(pb: State<'_, Arc<Playback>>, secs: f64) {
    pb.seek(secs);
}

#[tauri::command]
fn audio_volume(pb: State<'_, Arc<Playback>>, volume: f32) {
    pb.set_volume(volume);
}

#[tauri::command]
fn audio_state(pb: State<'_, Arc<Playback>>) -> PlaybackState {
    pb.state()
}

#[tauri::command]
fn reveal_in_folder(path: String) -> Result<(), String> {
    #[cfg(windows)]
    let result = std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .args(["-R", &path])
        .spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(Path::new(&path).parent().unwrap_or(Path::new(".")))
        .spawn();
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Jobs::default())
        .manage(Arc::new(PeaksStore::default()))
        .manage(Arc::new(playback::start_system()))
        .invoke_handler(tauri::generate_handler![
            default_folders,
            scan,
            repair,
            tool_status,
            plan_info,
            export_audio,
            ensure_peaks,
            get_peaks,
            get_samples,
            cancel_job,
            audio_devices,
            audio_device_details,
            audio_set_device,
            audio_load,
            audio_play,
            audio_pause,
            audio_seek,
            audio_volume,
            audio_state,
            reveal_in_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running Recording Fixer for Serato DJ");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_cannot_escape_the_output_folder() {
        assert_eq!(sanitize_stem("../../evil"), ".._.._evil");
        assert_eq!(sanitize_stem("a:b*c?"), "a_b_c_");
        assert_eq!(sanitize_stem("  "), "Serato Recording");
        assert_eq!(sanitize_stem("set."), "set");
    }

    fn parse_target(json: &str) -> Target {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn mp3_targets_map_to_codec_options() {
        let c = codec_for(parse_target(
            r#"{"kind":"mp3","mode":"vbr","value":2,"id3_version":4,"id3v1":true}"#,
        ))
        .unwrap();
        assert_eq!(
            c,
            Codec::Mp3(Mp3Options {
                mode: Mp3Mode::Vbr(2),
                id3_version: 4,
                id3v1: true
            })
        );
        let c = codec_for(parse_target(
            r#"{"kind":"mp3","mode":"abr","value":192,"id3_version":9,"id3v1":false}"#,
        ))
        .unwrap();
        assert_eq!(
            c,
            Codec::Mp3(Mp3Options {
                mode: Mp3Mode::Abr(192),
                id3_version: 3,
                id3v1: false
            })
        );
        let c = codec_for(parse_target(
            r#"{"kind":"mp3","mode":"cbr","value":320,"id3_version":3,"id3v1":false}"#,
        ))
        .unwrap();
        assert!(matches!(c, Codec::Mp3(o) if o.mode == Mp3Mode::Cbr(320)));
    }

    #[test]
    fn aac_targets_map_to_codec_options() {
        let c = codec_for(parse_target(
            r#"{"kind":"aac","encoder":"apple_coreaudio","mode":"vbr","value":9}"#,
        ))
        .unwrap();
        assert_eq!(
            c,
            Codec::Aac {
                encoder: APPLE_ENCODER.into(),
                mode: AacMode::Vbr(5)
            }
        );
        let c = codec_for(parse_target(
            r#"{"kind":"aac","encoder":"aac","mode":"cbr","value":256}"#,
        ))
        .unwrap();
        assert_eq!(
            c,
            Codec::Aac {
                encoder: "aac".into(),
                mode: AacMode::Cbr(256)
            }
        );
    }

    #[test]
    fn export_jobs_deserialize_with_partial_metadata() {
        let job: ExportJob = serde_json::from_str(
            r#"{"clips":[{"path":"a.aif","start_frame":0,"end_frame":null}],"target":{"kind":"aiff"},
                "output_dir":"out","file_stem":"x","metadata":{"title":"T"},"ffmpeg_path":null}"#,
        )
        .unwrap();
        assert_eq!(Metadata::from(job.metadata).pairs(), vec![("title", "T")]);
        assert_eq!(
            (job.fade_in_secs, job.fade_out_secs),
            (0.0, 0.0),
            "fades default to off"
        );
    }

    #[test]
    fn export_jobs_carry_fade_lengths() {
        let job: ExportJob = serde_json::from_str(
            r#"{"clips":[],"target":{"kind":"aiff"},"output_dir":"o","file_stem":"x",
                "ffmpeg_path":null,"fade_in_secs":2.5,"fade_out_secs":4}"#,
        )
        .unwrap();
        assert_eq!((job.fade_in_secs, job.fade_out_secs), (2.5, 4.0));
        let f = edit_core::Fade::from_secs(job.fade_in_secs, job.fade_out_secs, 44100.0);
        assert_eq!((f.in_frames, f.out_frames), (110_250, 176_400));
    }
}
