/**
 * Client-side view of the playback engine: polls its state, remembers the chosen output device,
 * and offers the transport actions. There is one engine, so there is one of these.
 */
import { api, emptyPlayback, type AudioDevice, type Clip, type DeviceSelection, type LoudnessJob, type OpenedDevice, type PlaybackState } from './api';

const KEY = 'audioDevice.v1';

export const defaultSelection = (): DeviceSelection => ({ id: null, out_pair: 0, buffer_frames: null });

export function loadSelection(storage: Pick<Storage, 'getItem'> | undefined = globalThis.localStorage): DeviceSelection {
  try {
    const raw = JSON.parse(storage?.getItem(KEY) ?? 'null');
    if (raw && typeof raw === 'object') {
      return {
        id: typeof raw.id === 'string' ? raw.id : null,
        out_pair: Number.isInteger(raw.out_pair) && raw.out_pair >= 0 ? raw.out_pair : 0,
        buffer_frames: Number.isInteger(raw.buffer_frames) && raw.buffer_frames > 0 ? raw.buffer_frames : null,
      };
    }
  } catch {
    /* unreadable settings fall back to the default device */
  }
  return defaultSelection();
}

function saveSelection(sel: DeviceSelection) {
  try {
    localStorage.setItem(KEY, JSON.stringify(sel));
  } catch {
    /* the choice just won't be remembered */
  }
}

/** Short human label for a device: "ASIO4ALL v2 (ASIO)". */
export function deviceLabel(d: { name: string; host: string }): string {
  return `${d.name} (${d.host.toUpperCase()})`;
}

/** The pairs of output channels a device offers: [{index: 0, label: "1-2"}, ...]. */
export function channelPairs(channels: number): { index: number; label: string }[] {
  const n = Math.max(1, Math.floor(channels / 2));
  return Array.from({ length: n }, (_, i) => ({ index: i, label: `${i * 2 + 1}-${i * 2 + 2}` }));
}

export class AudioStore {
  state = $state<PlaybackState>(emptyPlayback());
  devices = $state<AudioDevice[]>([]);
  selection = $state<DeviceSelection>(defaultSelection());
  /** What is loaded, for display: a file name, or "Editor". */
  label = $state('');
  /** Path of the file being previewed from the list ('' when the editor or nothing is loaded). */
  sourcePath = $state('');
  error = $state<string | null>(null);
  /** The device the engine actually opened (may differ from what was asked for). */
  opened = $state<OpenedDevice | null>(null);

  #timer: ReturnType<typeof setInterval> | undefined;

  start(intervalMs = 40) {
    if (this.#timer) return;
    this.#timer = setInterval(() => void this.poll(), intervalMs);
    void this.poll();
  }

  stop() {
    clearInterval(this.#timer);
    this.#timer = undefined;
  }

  async poll() {
    try {
      this.state = await api.audioState();
      if (this.state.device) this.opened = this.state.device;
    } catch {
      /* the engine is not reachable (e.g. during shutdown); keep the last state */
    }
  }

  async refreshDevices() {
    try {
      this.devices = await api.audioDevices();
    } catch (e) {
      this.error = String(e);
    }
  }

  /** Open the chosen device. Throws (and records `error`) if it cannot be opened. */
  async applySelection(sel: DeviceSelection): Promise<OpenedDevice> {
    this.error = null;
    try {
      const opened = await api.audioSetDevice(sel);
      this.selection = sel;
      this.opened = opened;
      saveSelection(sel);
      await this.poll();
      return opened;
    } catch (e) {
      this.error = String(e);
      throw e;
    }
  }

  /** Re-open the remembered device at startup. A missing device falls back to the system default. */
  async restore(): Promise<void> {
    const saved = loadSelection();
    this.selection = saved;
    await this.refreshDevices();
    if (saved.id === null) return; // the default opens on first use
    try {
      await this.applySelection(saved);
    } catch {
      this.selection = defaultSelection();
    }
  }

  async load(clips: Clip[], startSecs: number, label: string, loudness: LoudnessJob | null = null) {
    this.error = null;
    try {
      const info = await api.audioLoad(clips, startSecs, loudness);
      this.label = label;
      this.sourcePath = '';
      await this.poll();
      return info;
    } catch (e) {
      this.error = String(e);
      throw e;
    }
  }

  /** Load one whole file and start playing it from the beginning. */
  async preview(path: string, label: string) {
    await this.load([{ path, start_frame: 0, end_frame: null }], 0, label);
    this.sourcePath = path;
    await this.play();
  }

  async play() {
    // Only show "playing" ahead of the engine when it has something to play; otherwise the next poll would flip it back.
    if (this.state.loaded) this.state = { ...this.state, playing: true, ended: false };
    await api.audioPlay();
  }

  async pause() {
    this.state = { ...this.state, playing: false };
    await api.audioPause();
  }

  async toggle() {
    if (!this.state.loaded) return;
    await (this.state.playing ? this.pause() : this.play());
  }

  async seek(secs: number) {
    const clamped = Math.min(Math.max(0, secs), this.state.duration_secs || secs);
    this.state = { ...this.state, position_secs: clamped, ended: false };
    await api.audioSeek(clamped);
  }

  async setVolume(v: number) {
    this.state = { ...this.state, volume: v };
    await api.audioVolume(v);
  }
}

export const audio = new AudioStore();
