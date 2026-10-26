//! Thin Tauri command layer. All real work lives in the crates under `crates/`.

mod logging;
mod update;

use aiff_core::inuse::{is_in_use, RECENT_WINDOW};
use dynamics::leveler::Leveling;
use dynamics::maximizer::{self, Maximizer, Mode};
use dynamics::pcm::PcmFormat;
use edit_core::{Clip, FadeReader};
use export::{AacMode, Codec, ExportSpec, Metadata, Mp3Mode, Mp3Options, APPLE_ENCODER};
use library::{FileEntry, FolderInfo};
use peaks::{cache, Peaks};
use playback::{DeviceInfo, DeviceSelection, LoadInfo, OpenedInfo, Playback, PlaybackState};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindowBuilder};

/// Where the log file lives, for the Settings window to show or read.
struct LogDir(PathBuf);

/// Log a failed command with what it was doing, then pass the error on to the UI unchanged.
fn logged<T>(what: &str, result: Result<T, String>) -> Result<T, String> {
    if let Err(e) = &result {
        log::error!("{what} failed: {e}");
    }
    result
}

/// Only the file name: logs get pasted into bug reports, and folders can name people.
fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

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
    log::info!("repair: {} file(s), in place: {in_place}", paths.len());
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| match repair_one(&app, &cancel, &path, in_place) {
                Ok(output) => {
                    log::info!("repaired {} -> {}", file_label(&path), file_label(&output));
                    RepairResult {
                        path,
                        ok: true,
                        output: Some(output),
                        error: None,
                    }
                }
                Err(error) => {
                    log::error!("repair of {} failed: {error}", file_label(&path));
                    RepairResult {
                        path,
                        ok: false,
                        output: None,
                        error: Some(error),
                    }
                }
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
            log::warn!("ffmpeg was not found");
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
        let encoders = export::probe_encoders(&path).unwrap_or_else(|e| {
            log::warn!("could not list ffmpeg encoders: {e}");
            Default::default()
        });
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
        log::info!(
            "ffmpeg: {}; AAC encoders: {}",
            path.display(),
            aac.join(", ")
        );
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
    let result = tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| e.to_string())?;
    logged("planning the edit", result)
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

/// Loudness maximizer settings from the export window; absent means the audio is not processed.
#[derive(Deserialize)]
struct LoudnessDto {
    /// "lufs" (reach a loudness, leveling as chosen) or "peak" (normalize the highest peak).
    mode: String,
    lufs: f64,
    ceiling_db: f64,
    /// "off", "gentle", "medium" or "strong".
    leveling: String,
}

impl LoudnessDto {
    fn params(&self) -> maximizer::Params {
        maximizer::Params {
            mode: if self.mode == "peak" {
                Mode::Peak
            } else {
                Mode::Loudness(self.lufs.clamp(-40.0, -3.0))
            },
            ceiling_db: self.ceiling_db.clamp(-20.0, 0.0),
            leveling: match self.leveling.as_str() {
                "gentle" => Leveling::gentle(),
                "strong" => Leveling::strong(),
                "off" => Leveling::OFF,
                _ => Leveling::medium(),
            },
            precise: true,
        }
    }
}

#[derive(Serialize)]
struct ExportOutcome {
    path: String,
    /// One plain sentence about what the loudness processing did, when it ran.
    summary: Option<String>,
}

fn describe_report(r: &maximizer::Report) -> String {
    let limiter = if r.limiter.max_reduction_db < 0.1 {
        "the limiter did not need to act".to_string()
    } else {
        format!(
            "the limiter averaged {:.1} dB and reached {:.1} dB at most",
            r.limiter.mean_reduction_db, r.limiter.max_reduction_db
        )
    };
    format!(
        "Loudness {:.1} LUFS (was {:.1}), peak {:.1} dBTP; {limiter}",
        r.output.lufs, r.input.lufs, r.output.true_peak_db
    )
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
    #[serde(default)]
    loudness: Option<LoudnessDto>,
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

fn run_export(
    app: &AppHandle,
    cancel: &AtomicBool,
    job: ExportJob,
) -> Result<ExportOutcome, String> {
    let plan = edit_core::plan(&to_clips(&job.clips)).map_err(|e| e.to_string())?;
    let stem = sanitize_stem(&job.file_stem);
    let ext = match &job.target {
        Target::Aiff => "aif",
        Target::Mp3 { .. } => "mp3",
        Target::Aac { .. } => "m4a",
    };
    let out = library::unique_path(Path::new(&job.output_dir), &stem, "", ext);
    let fade =
        edit_core::Fade::from_secs(job.fade_in_secs, job.fade_out_secs, plan.format.sample_rate);

    // Check the encoder before spending time on anything else.
    let encode = if matches!(job.target, Target::Aiff) {
        None
    } else {
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
        Some((codec, ffmpeg))
    };

    // Loudness: measure first (and rehearse once), so the real pass knows what to do.
    let fmt = PcmFormat {
        bits: plan.format.bits,
        channels: plan.format.channels,
        rate: plan.format.sample_rate,
    };
    let prepared = match &job.loudness {
        None => None,
        Some(l) => {
            let mut measuring = Throttle::new(app, format!("Measuring loudness of {stem}"));
            let mut checking =
                Throttle::new(app, format!("Checking the loudness result for {stem}"));
            let prepared = maximizer::prepare(
                || Ok(plan.reader()),
                fmt,
                plan.total_frames,
                &l.params(),
                &mut |phase, done, total| {
                    match phase {
                        maximizer::Phase::Measuring => measuring.emit(done, total),
                        maximizer::Phase::Rehearsing => checking.emit(done, total),
                    }
                    !cancel.load(Ordering::Relaxed)
                },
            )
            .map_err(|e| e.to_string())?;
            log::info!(
                "loudness plan for {stem}: input {:.1} LUFS, static gain {:+.1} dB",
                prepared.input.lufs,
                prepared.curve.static_db
            );
            Some(prepared)
        }
    };

    let mut throttle = Throttle::new(app, format!("Exporting {stem}.{ext}"));
    let mut progress = |done: u64, total: u64| {
        throttle.emit(done, total);
        !cancel.load(Ordering::Relaxed)
    };

    // read -> [loudness] -> fades -> encode. Fades come after the leveller so it cannot lift a fade-out.
    let maximizer = prepared.as_ref().map(|p| Maximizer::new(plan.reader(), p));
    let report = maximizer.as_ref().map(Maximizer::report_handle);
    let inner: Box<dyn Read> = match maximizer {
        Some(m) => Box::new(m),
        None => Box::new(plan.reader()),
    };
    let mut source = FadeReader::new(
        inner,
        fade,
        plan.format.bits,
        plan.format.channels,
        plan.total_frames,
    );

    match encode {
        None => plan
            .write_aiff_from(&out, &mut source, &mut progress)
            .map_err(|e| e.to_string())?,
        Some((codec, ffmpeg)) => {
            let spec = ExportSpec {
                format: plan.format.clone(),
                codec,
                output: out.clone(),
                metadata: job.metadata.into(),
            };
            export::run(
                &ffmpeg,
                &spec,
                &mut source,
                plan.total_bytes(),
                &mut progress,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let summary = report
        .and_then(|r| r.lock().ok().and_then(|mut r| r.take()))
        .map(|r| describe_report(&r));
    if let Some(s) = &summary {
        log::info!("{stem}: {s}");
    }
    Ok(ExportOutcome {
        path: out.display().to_string(),
        summary,
    })
}

#[tauri::command]
async fn export_audio(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job: ExportJob,
) -> Result<ExportOutcome, String> {
    let cancel = jobs.cancel.clone();
    cancel.store(false, Ordering::Relaxed);
    log::info!(
        "export: {} clip(s) as {}",
        job.clips.len(),
        match &job.target {
            Target::Aiff => "AIFF".to_string(),
            Target::Mp3 { mode, value, .. } => format!("MP3 {mode} {value}"),
            Target::Aac {
                encoder,
                mode,
                value,
            } => format!("AAC {encoder} {mode} {value}"),
        }
    );
    let result = tauri::async_runtime::spawn_blocking(move || run_export(&app, &cancel, job))
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(out) = &result {
        log::info!("export finished: {}", file_label(&out.path));
    }
    logged("export", result)
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
    let result = tauri::async_runtime::spawn_blocking(move || {
        peaks_for(&app, &store, &path).map(|p| PeakInfo { frames: p.frames })
    })
    .await
    .map_err(|e| e.to_string())?;
    logged("building the waveform", result)
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
    let result = tauri::async_runtime::spawn_blocking(move || {
        let peaks = peaks_for(&app, &store, &path)?;
        let bytes: Vec<u8> = peaks
            .columns(start, end, width)
            .into_iter()
            .map(|v| v as u8)
            .collect();
        Ok::<_, String>(tauri::ipc::Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?;
    logged("reading waveform peaks", result)
}

/// Exact 16-bit `[min, max]` per frame for frames `[start, end)` (little-endian i16 pairs), for
/// zooming in to individual samples. At most `peaks::MAX_SAMPLE_FRAMES` (262,144) frames per call.
#[tauri::command]
async fn get_samples(path: String, start: u64, end: u64) -> Result<tauri::ipc::Response, String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let v = peaks::read_samples(Path::new(&path), start, end).map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = v.into_iter().flat_map(i16::to_le_bytes).collect();
        Ok::<_, String>(tauri::ipc::Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?;
    logged("reading samples", result)
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
    let result = tauri::async_runtime::spawn_blocking(move || {
        pb.set_device(selection).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Ok(opened) = &result {
        log::info!(
            "audio output: {} ({}), {} Hz",
            opened.name,
            opened.host,
            opened.rate
        );
    }
    logged("opening the audio device", result)
}

#[tauri::command]
async fn audio_load(
    pb: State<'_, Arc<Playback>>,
    clips: Vec<ClipDto>,
    start_secs: f64,
) -> Result<LoadInfo, String> {
    let pb = pb.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        pb.load(to_clips(&clips), start_secs)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    logged("loading audio for playback", result)
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

#[tauri::command]
fn log_location(dir: State<'_, LogDir>) -> String {
    logging::log_file(&dir.0).display().to_string()
}

/// The end of the log, for the user to copy into a bug report.
#[tauri::command]
fn read_log(dir: State<'_, LogDir>) -> String {
    logging::read_tail(&dir.0, 256 * 1024)
}

/// Lets the UI record its own errors (uncaught exceptions) in the same file.
#[tauri::command]
fn log_ui(level: String, message: String) {
    let message: String = message.chars().take(4000).collect();
    match level.as_str() {
        "warn" => log::warn!(target: "recording_fixer::ui", "{message}"),
        "info" => log::info!(target: "recording_fixer::ui", "{message}"),
        _ => log::error!(target: "recording_fixer::ui", "{message}"),
    }
}

/// Asks GitHub's public release list whether a newer version exists. See update.rs and PRIVACY.md.
#[tauri::command]
async fn check_for_update(app: AppHandle) -> Result<update::UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || update::check(&current))
        .await
        .map_err(|e| e.to_string())?;
    match &result {
        Ok(info) if info.newer => log::info!("update check: {:?} is available", info.latest),
        Ok(_) => log::info!("update check: up to date"),
        Err(e) => log::warn!("update check failed: {e}"),
    }
    result
}

/// The window is described in tauri.conf.json (`create: false`) and built here so WebDriver can work.
/// tauri-driver asks for the browser's debug port through WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, but
/// wry always sets its own argument list on the WebView2 environment, and on the CI runner the
/// variable was then ignored (no DevToolsActivePort, so no session). Folding the variable into the
/// arguments we pass ourselves removes the dependence on that precedence. Unset, nothing changes.
fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .first()
        .expect("tauri.conf.json defines the main window");
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut builder = WebviewWindowBuilder::from_config(app, config)?;
    #[cfg(windows)]
    if let Some(extra) = std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
        // wry's own defaults, which setting the arguments replaces.
        let args = format!(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required {}",
            extra.to_string_lossy()
        );
        builder = builder.additional_browser_args(&args);
    }
    builder.build()?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Jobs::default())
        .manage(Arc::new(PeaksStore::default()))
        .manage(Arc::new(playback::start_system()))
        .setup(|app| {
            // Logging first, so anything that goes wrong while starting is recorded.
            let log_dir = app
                .path()
                .app_log_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("recording-fixer-logs"));
            if let Err(e) = logging::init(&log_dir) {
                eprintln!("could not open the log file in {}: {e}", log_dir.display());
            }
            app.manage(LogDir(log_dir));
            create_main_window(app.handle())?;
            Ok(())
        })
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
            reveal_in_folder,
            log_location,
            read_log,
            log_ui,
            check_for_update
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

    fn loudness(json: &str) -> LoudnessDto {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn loudness_settings_map_to_the_maximizer_and_stay_in_range() {
        let p =
            loudness(r#"{"mode":"lufs","lufs":-14,"ceiling_db":-1,"leveling":"strong"}"#).params();
        assert_eq!(p.mode, Mode::Loudness(-14.0));
        assert_eq!(p.ceiling_db, -1.0);
        assert_eq!(p.leveling, Leveling::strong());
        assert!(p.precise);
        // Nonsense from the UI is pulled into range, and unknown names fall back to the default.
        let p = loudness(r#"{"mode":"lufs","lufs":12,"ceiling_db":3,"leveling":"wild"}"#).params();
        assert_eq!(p.mode, Mode::Loudness(-3.0));
        assert_eq!(p.ceiling_db, 0.0);
        assert_eq!(p.leveling, Leveling::medium());
        assert_eq!(
            loudness(r#"{"mode":"peak","lufs":-14,"ceiling_db":-0.1,"leveling":"off"}"#)
                .params()
                .mode,
            Mode::Peak
        );
    }

    #[test]
    fn export_jobs_without_loudness_still_deserialize() {
        let job: ExportJob = serde_json::from_str(
            r#"{"clips":[],"target":{"kind":"aiff"},"output_dir":"x","file_stem":"y","ffmpeg_path":null}"#,
        )
        .unwrap();
        assert!(job.loudness.is_none());
    }
}
