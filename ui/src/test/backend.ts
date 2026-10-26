// A scriptable fake of the Rust backend, installed behind the mocked Tauri `invoke`.
import type { Mock } from 'vitest';
import {
  emptyPlayback,
  type AudioDevice,
  type FileEntry,
  type PlaybackState,
  type ToolStatus,
  type UpdateInfo,
} from '../lib/api';

export function entry(over: Partial<FileEntry> & { name: string }): FileEntry {
  return {
    path: `C:/rec/${over.name}`,
    folder: 'C:/rec',
    folder_kind: 'recording',
    size: 100_000_000,
    modified_ms: Date.parse('2025-10-04T22:00:00'),
    status: 'ok',
    detail: null,
    duration_secs: 600,
    sample_rate: 44100,
    bits: 16,
    channels: 2,
    title: null,
    recorded: null,
    ...over,
  };
}

export const SAMPLE_FILES: FileEntry[] = [
  entry({ name: 'Serato Recording 1.aif', duration_secs: 10800, size: 1_905_124_985, recorded: '2024-08-03 21:58:12', modified_ms: Date.parse('2024-08-04T00:58:00') }),
  entry({ name: 'Serato Recording 2.aif', duration_secs: 4800, size: 846_744_185, recorded: '2025-10-04 21:11:51', modified_ms: Date.parse('2025-10-04T22:31:00') }),
  entry({
    name: 'Serato Recording 5.aif',
    folder: 'C:/rec temp',
    folder_kind: 'temp',
    status: 'broken',
    detail: 'length is missing; file size field is wrong',
    duration_secs: 1148.4,
    sample_rate: 48000,
    modified_ms: Date.parse('2026-08-15T23:14:00'),
  }),
  entry({ name: 'Serato Recording 6.aif', folder: 'C:/rec temp', folder_kind: 'temp', status: 'inuse', detail: 'Serato may still be recording this file', duration_secs: 162 }),
];

export const SAMPLE_TOOL: ToolStatus = {
  ffmpeg_path: 'C:/ffmpeg.exe',
  ffmpeg_version: 'ffmpeg version 9.0.2-full_build',
  aac_encoders: ['apple_coreaudio', 'aac', 'aac_mf'],
  best_aac: 'apple_coreaudio',
  mp3: true,
  apple: 'Apple AAC (CoreAudioToolbox)',
};

export const SAMPLE_DEVICES: AudioDevice[] = [
  { id: 'asio:ASIO4ALL v2', host: 'asio', name: 'ASIO4ALL v2', channels: 0, default_rate: 0, is_default: true },
  { id: 'asio:DJM-S7 ASIO', host: 'asio', name: 'DJM-S7 ASIO', channels: 0, default_rate: 0, is_default: false },
  { id: 'wasapi:speakers', host: 'wasapi', name: 'Speakers', channels: 2, default_rate: 48000, is_default: true },
];

export interface BackendConfig {
  files?: FileEntry[];
  tool?: ToolStatus;
  devices?: AudioDevice[];
  /** Channel count reported by `audio_device_details`. */
  detailChannels?: number;
  /** Default folders (by path) that should be reported as not existing. */
  missingFolders?: string[];
  /** What check_for_update answers (default: up to date). */
  update?: UpdateInfo;
  /** Make named commands fail with this message. */
  failures?: Record<string, string>;
}

export interface Backend {
  /** Every command issued, in order. */
  calls: { cmd: string; args: Record<string, unknown> }[];
  of(cmd: string): Record<string, unknown>[];
  playback: PlaybackState;
  /** How long udio_load reports the loaded material to be (default: 100 s per clip). */
  durationOverride?: number;
  config: Required<Pick<BackendConfig, 'files' | 'tool' | 'devices'>>;
  fail(cmd: string, message: string | null): void;
}

export function installBackend(invoke: Mock, cfg: BackendConfig = {}): Backend {
  const failures = { ...cfg.failures };
  const backend: Backend = {
    calls: [],
    of: (cmd) => backend.calls.filter((c) => c.cmd === cmd).map((c) => c.args),
    playback: { ...emptyPlayback() },
    config: { files: cfg.files ?? SAMPLE_FILES, tool: cfg.tool ?? SAMPLE_TOOL, devices: cfg.devices ?? SAMPLE_DEVICES },
    fail: (cmd, message) => {
      if (message === null) delete failures[cmd];
      else failures[cmd] = message;
    },
  };
  const P = () => backend.playback;

  invoke.mockImplementation(async (cmd: string, args: Record<string, any> = {}) => {
    backend.calls.push({ cmd, args });
    if (failures[cmd]) throw failures[cmd];
    switch (cmd) {
      case 'default_folders':
        return [
          { path: 'C:/rec', label: 'Recording', kind: 'recording', exists: !cfg.missingFolders?.includes('C:/rec') },
          { path: 'C:/rec temp', label: 'Recording temp', kind: 'temp', exists: !cfg.missingFolders?.includes('C:/rec temp') },
        ];
      case 'scan':
        return backend.config.files;
      case 'tool_status':
        return backend.config.tool;
      case 'repair':
        return (args.paths as string[]).map((path) => ({ path, ok: true, output: path.replace('.aif', ' (fixed).aif'), error: null }));
      case 'plan_info':
        return { total_frames: 1, duration_secs: 4000, bytes: 1_000_000, format: '44100 Hz, 16-bit, 2 ch', fits_aiff: true };
      case 'export_audio':
        return {
          path: `${args.job.output_dir}/${args.job.file_stem}.out`,
          summary: args.job.loudness ? 'Loudness -14.0 LUFS (was -19.6), peak -1.0 dBTP; the limiter did not need to act' : null,
        };
      case 'ensure_peaks':
        return { frames: 1 };
      case 'get_peaks': {
        const n = args.width as number;
        const buf = new Int8Array(n * 2);
        for (let i = 0; i < n; i++) {
          buf[2 * i] = -60;
          buf[2 * i + 1] = 60;
        }
        return buf.buffer;
      }
      case 'get_samples': {
        // A ramp, so tests can tell frames apart: frame f has min = f, max = f + 1.
        const s = args.start as number;
        const n = Math.max(0, (args.end as number) - s);
        const out = new Int16Array(n * 2);
        for (let i = 0; i < n; i++) {
          out[2 * i] = s + i;
          out[2 * i + 1] = s + i + 1;
        }
        return out.buffer;
      }
      case 'check_for_update':
        return cfg.update ?? { current: '0.1.0', latest: '0.1.0', url: 'https://github.com/djryanj/recording-fixer/releases/tag/v0.1.0', newer: false };
      case 'log_location':
        return 'C:/logs/recording-fixer.log';
      case 'read_log':
        return '2026-01-01T00:00:00Z INFO  starting\n';
      case 'log_ui':
        return null;
      case 'plugin:app|version':
        return '0.1.0';
      case 'plugin:opener|open_url':
        return null;
      case 'audio_devices':
        return backend.config.devices;
      case 'audio_device_details':
        return { id: args.id, host: 'asio', name: 'detail', channels: cfg.detailChannels ?? 8, default_rate: 44100, is_default: false };
      case 'audio_set_device':
        return { id: args.selection.id ?? 'wasapi:speakers', host: 'asio', name: 'Opened device', rate: 44100, channels: 8, out_pair: args.selection.out_pair, buffer_frames: args.selection.buffer_frames };
      case 'audio_load': {
        const clips = args.clips as { path: string; start_frame: number; end_frame: number | null }[];
        P().loaded = true;
        P().duration_secs = backend.durationOverride ?? clips.length * 100;
        P().position_secs = args.startSecs as number;
        return {
          duration_secs: P().duration_secs,
          total_frames: 1,
          rate: 44100,
          channels: 2,
          bits: 16,
        };
      }
      case 'audio_play':
        P().playing = true;
        return null;
      case 'audio_pause':
        P().playing = false;
        return null;
      case 'audio_seek':
        P().position_secs = args.secs as number;
        return null;
      case 'audio_volume':
        P().volume = args.volume as number;
        return null;
      case 'audio_state':
        return { ...P() };
      case 'loudness_plan':
        return { input_lufs: -19.6, static_gain_db: 5.6, gain_min_db: 3.4, gain_max_db: 9.0, gain_db: Array.from({ length: 11 }, (_, i) => 5 + (i % 4)) };
      case 'ensure_loudness':
        return null;
      case 'get_loudness': {
        // A step in the middle, so a test can see the graph follow the data.
        const n = args.width as number;
        const out = new Float32Array(n);
        for (let i = 0; i < n; i++) out[i] = i < n / 2 ? -24 : -18;
        return out.buffer;
      }
      case 'cancel_job':
      case 'reveal_in_folder':
        return null;
      default:
        throw new Error(`test backend: unhandled command ${cmd}`);
    }
  });
  return backend;
}
