import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type Status = 'ok' | 'broken' | 'inuse' | 'unreadable' | 'unsupported';
export type FolderKind = 'recording' | 'temp' | 'other';

export interface FileEntry {
  path: string;
  name: string;
  folder: string;
  folder_kind: FolderKind;
  size: number;
  modified_ms: number;
  status: Status;
  detail: string | null;
  duration_secs: number | null;
  sample_rate: number | null;
  bits: number | null;
  channels: number | null;
  title: string | null;
  recorded: string | null;
}

export interface FolderInfo {
  path: string;
  label: string;
  kind: FolderKind;
  exists: boolean;
}

export interface ToolStatus {
  ffmpeg_path: string | null;
  ffmpeg_version: string | null;
  aac_encoders: string[];
  best_aac: string | null;
  mp3: boolean;
  apple: string | null;
}

export interface UpdateInfo {
  current: string;
  /** Newest published release; null if the project has not published one yet. */
  latest: string | null;
  url: string | null;
  newer: boolean;
}

export interface Clip {
  path: string;
  start_frame: number;
  end_frame: number | null;
}

export interface PlanInfo {
  total_frames: number;
  duration_secs: number;
  bytes: number;
  format: string;
  fits_aiff: boolean;
}

export type Target =
  | { kind: 'aiff' }
  | { kind: 'mp3'; mode: 'cbr' | 'vbr' | 'abr'; value: number; id3_version: 3 | 4; id3v1: boolean }
  | { kind: 'aac'; encoder: string; mode: 'cbr' | 'vbr'; value: number };

export interface Metadata {
  title: string;
  artist: string;
  album: string;
  album_artist: string;
  genre: string;
  date: string;
  comment: string;
  track: string;
}

export const emptyMetadata = (): Metadata => ({ title: '', artist: '', album: '', album_artist: '', genre: '', date: '', comment: '', track: '' });

export interface RepairResult {
  path: string;
  ok: boolean;
  output: string | null;
  error: string | null;
}

/** Loudness maximizer settings for one export; null leaves the audio untouched. */
export interface LoudnessJob {
  /** "lufs": reach a loudness, evening out as chosen. "peak": raise the whole thing until its highest peak meets the limit. */
  mode: 'lufs' | 'peak';
  lufs: number;
  ceiling_db: number;
  leveling: 'off' | 'gentle' | 'medium' | 'strong';
}

export interface ExportOutcome {
  path: string;
  /** One sentence on what the loudness processing did, when it ran. */
  summary: string | null;
}

export interface ExportJob {
  clips: Clip[];
  target: Target;
  output_dir: string;
  /** Final file name without extension (templates already expanded). */
  file_stem: string;
  metadata: Metadata;
  ffmpeg_path: string | null;
  /** Fade in over the start and out over the end of the result, in seconds (0 = none). */
  fade_in_secs: number;
  fade_out_secs: number;
  loudness: LoudnessJob | null;
}

export interface Progress {
  label: string;
  done: number;
  total: number;
}

export interface AudioDevice {
  id: string;
  host: string;
  name: string;
  channels: number;
  default_rate: number;
  is_default: boolean;
}

export interface DeviceSelection {
  id: string | null;
  out_pair: number;
  buffer_frames: number | null;
}

export interface OpenedDevice {
  id: string;
  host: string;
  name: string;
  rate: number;
  channels: number;
  out_pair: number;
  buffer_frames: number | null;
}

export interface LoadInfo {
  duration_secs: number;
  total_frames: number;
  rate: number;
  channels: number;
  bits: number;
}

export interface PlaybackState {
  loaded: boolean;
  playing: boolean;
  ended: boolean;
  position_secs: number;
  duration_secs: number;
  peak_l: number;
  peak_r: number;
  volume: number;
  underruns: number;
  device: OpenedDevice | null;
  error: string | null;
}

export const emptyPlayback = (): PlaybackState => ({
  loaded: false,
  playing: false,
  ended: false,
  position_secs: 0,
  duration_secs: 0,
  peak_l: 0,
  peak_r: 0,
  volume: 1,
  underruns: 0,
  device: null,
  error: null,
});

export const api = {
  audioDevices: () => invoke<AudioDevice[]>('audio_devices'),
  audioDeviceDetails: (id: string) => invoke<AudioDevice | null>('audio_device_details', { id }),
  audioSetDevice: (selection: DeviceSelection) => invoke<OpenedDevice>('audio_set_device', { selection }),
  audioLoad: (clips: Clip[], startSecs: number) => invoke<LoadInfo>('audio_load', { clips, startSecs }),
  audioPlay: () => invoke<void>('audio_play'),
  audioPause: () => invoke<void>('audio_pause'),
  audioSeek: (secs: number) => invoke<void>('audio_seek', { secs }),
  audioVolume: (volume: number) => invoke<void>('audio_volume', { volume }),
  audioState: () => invoke<PlaybackState>('audio_state'),
  defaultFolders: () => invoke<FolderInfo[]>('default_folders'),
  scan: (folders: string[]) => invoke<FileEntry[]>('scan', { folders }),
  repair: (paths: string[], inPlace: boolean) => invoke<RepairResult[]>('repair', { paths, inPlace }),
  toolStatus: (userPath: string | null) => invoke<ToolStatus>('tool_status', { userPath }),
  planInfo: (clips: Clip[]) => invoke<PlanInfo>('plan_info', { clips }),
  exportAudio: (job: ExportJob) => invoke<ExportOutcome>('export_audio', { job }),
  ensurePeaks: (path: string) => invoke<{ frames: number }>('ensure_peaks', { path }),
  /** `width` columns of [min, max] signed bytes for frames [start, end). */
  getPeaks: async (path: string, start: number, end: number, width: number) =>
    new Int8Array(await invoke<ArrayBuffer>('get_peaks', { path, start, end, width })),
  /** Exact 16-bit [min, max] per frame for frames [start, end) (at most 262,144 frames). */
  getSamples: async (path: string, start: number, end: number) => new Int16Array(await invoke<ArrayBuffer>('get_samples', { path, start, end })),
  checkForUpdate: () => invoke<UpdateInfo>('check_for_update'),
  logLocation: () => invoke<string>('log_location'),
  readLog: () => invoke<string>('read_log'),
  logUi: (level: 'error' | 'warn' | 'info', message: string) => invoke<void>('log_ui', { level, message }),
  cancelJob: () => invoke<void>('cancel_job'),
  reveal: (path: string) => invoke<void>('reveal_in_folder', { path }),
  onProgress: (fn: (p: Progress) => void) => listen<Progress>('job-progress', (e) => fn(e.payload)),
};
