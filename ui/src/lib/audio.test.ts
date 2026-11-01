import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { emptyPlayback, type PlaybackState } from './api';

vi.mock('./api', async (orig) => {
  const real = await orig<typeof import('./api')>();
  return {
    ...real,
    api: {
      audioState: vi.fn(),
      audioDevices: vi.fn(),
      audioSetDevice: vi.fn(),
      audioLoad: vi.fn(),
      audioPlay: vi.fn(),
      audioPause: vi.fn(),
      audioSeek: vi.fn(),
      audioVolume: vi.fn(),
    },
  };
});

import { api } from './api';
import { AudioStore, channelPairs, defaultSelection, deviceLabel, loadSelection } from './audio.svelte';

const mocked = api as unknown as Record<keyof typeof api, ReturnType<typeof vi.fn>>;

function state(over: Partial<PlaybackState> = {}): PlaybackState {
  return { ...emptyPlayback(), ...over };
}

class MemoryStorage {
  data = new Map<string, string>();
  getItem = (k: string) => this.data.get(k) ?? null;
  setItem = (k: string, v: string) => void this.data.set(k, v);
}

beforeEach(() => {
  vi.stubGlobal('localStorage', new MemoryStorage());
  for (const fn of Object.values(mocked)) fn.mockReset();
  mocked.audioState.mockResolvedValue(state());
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('helpers', () => {
  it('labels devices and channel pairs', () => {
    expect(deviceLabel({ name: 'ASIO4ALL v2', host: 'asio' })).toBe('ASIO4ALL v2 (ASIO)');
    expect(channelPairs(2)).toEqual([{ index: 0, label: '1-2' }]);
    expect(channelPairs(8).map((p) => p.label)).toEqual(['1-2', '3-4', '5-6', '7-8']);
    expect(channelPairs(0)).toHaveLength(1);
    expect(channelPairs(1)).toHaveLength(1);
  });

  it('loads a saved selection and sanitises junk', () => {
    const s = new MemoryStorage();
    expect(loadSelection(s)).toEqual(defaultSelection());
    s.setItem('audioDevice.v1', JSON.stringify({ id: 'asio:X', out_pair: 2, buffer_frames: 256 }));
    expect(loadSelection(s)).toEqual({ id: 'asio:X', out_pair: 2, buffer_frames: 256 });
    s.setItem('audioDevice.v1', JSON.stringify({ id: 5, out_pair: -1, buffer_frames: 'big' }));
    expect(loadSelection(s)).toEqual(defaultSelection());
    s.setItem('audioDevice.v1', '{not json');
    expect(loadSelection(s)).toEqual(defaultSelection());
  });
});

describe('AudioStore polling', () => {
  it('polls the engine on a timer until stopped', async () => {
    vi.useFakeTimers();
    mocked.audioState.mockResolvedValue(state({ loaded: true, position_secs: 3 }));
    const a = new AudioStore();
    a.start(40);
    await vi.advanceTimersByTimeAsync(130);
    expect(a.state.position_secs).toBe(3);
    const calls = mocked.audioState.mock.calls.length;
    expect(calls).toBeGreaterThanOrEqual(3);
    a.stop();
    await vi.advanceTimersByTimeAsync(400);
    expect(mocked.audioState.mock.calls.length).toBe(calls);
  });

  it('starting twice does not double the polling', async () => {
    vi.useFakeTimers();
    const a = new AudioStore();
    a.start(40);
    a.start(40);
    await vi.advanceTimersByTimeAsync(100);
    expect(mocked.audioState.mock.calls.length).toBeLessThanOrEqual(4);
    a.stop();
  });

  it('keeps the last state when the engine cannot be reached', async () => {
    const a = new AudioStore();
    mocked.audioState.mockResolvedValueOnce(state({ position_secs: 7 }));
    await a.poll();
    mocked.audioState.mockRejectedValueOnce(new Error('gone'));
    await a.poll();
    expect(a.state.position_secs).toBe(7);
  });
});

describe('transport', () => {
  it('toggle plays and pauses, updating the state immediately', async () => {
    const a = new AudioStore();
    a.state = state({ loaded: true });
    await a.toggle();
    expect(a.state.playing).toBe(true);
    expect(mocked.audioPlay).toHaveBeenCalledTimes(1);
    await a.toggle();
    expect(a.state.playing).toBe(false);
    expect(mocked.audioPause).toHaveBeenCalledTimes(1);
  });

  it('toggle does nothing when nothing is loaded', async () => {
    const a = new AudioStore();
    await a.toggle();
    expect(mocked.audioPlay).not.toHaveBeenCalled();
  });

  it('play does not claim to be playing when nothing is loaded (e.g. no audio device)', async () => {
    // Showing Pause until the next poll made the e2e suite see playback that never happened.
    const a = new AudioStore();
    await a.play();
    expect(a.state.playing).toBe(false);
  });

  it('seek clamps to the material and reports the new position at once', async () => {
    const a = new AudioStore();
    a.state = state({ loaded: true, duration_secs: 100 });
    await a.seek(250);
    expect(a.state.position_secs).toBe(100);
    expect(mocked.audioSeek).toHaveBeenLastCalledWith(100);
    await a.seek(-4);
    expect(a.state.position_secs).toBe(0);
    expect(mocked.audioSeek).toHaveBeenLastCalledWith(0);
  });

  it('seek clears the ended flag so the UI does not show a finished transport', async () => {
    const a = new AudioStore();
    a.state = state({ loaded: true, ended: true, duration_secs: 10 });
    await a.seek(2);
    expect(a.state.ended).toBe(false);
  });

  it('preview loads the whole file from the start and plays it', async () => {
    const a = new AudioStore();
    mocked.audioLoad.mockResolvedValue({ duration_secs: 5, total_frames: 1, rate: 44100, channels: 2, bits: 16 });
    await a.preview('C:/r/a.aif', 'a.aif');
    expect(mocked.audioLoad).toHaveBeenCalledWith([{ path: 'C:/r/a.aif', start_frame: 0, end_frame: null }], 0, null);
    expect(mocked.audioPlay).toHaveBeenCalled();
    expect(a.label).toBe('a.aif');
  });

  it('a failed load is recorded and rethrown, and nothing plays', async () => {
    const a = new AudioStore();
    mocked.audioLoad.mockRejectedValue('no such file');
    await expect(a.preview('x', 'x')).rejects.toBe('no such file');
    expect(a.error).toBe('no such file');
    expect(mocked.audioPlay).not.toHaveBeenCalled();
  });

  it('volume is sent to the engine and shown immediately', async () => {
    const a = new AudioStore();
    await a.setVolume(0.4);
    expect(a.state.volume).toBe(0.4);
    expect(mocked.audioVolume).toHaveBeenCalledWith(0.4);
  });
});

describe('device selection', () => {
  const sel = { id: 'asio:ASIO4ALL v2', out_pair: 1, buffer_frames: 256 };
  const opened = { id: sel.id, host: 'asio', name: 'ASIO4ALL v2', rate: 44100, channels: 4, out_pair: 1, buffer_frames: 256 };

  it('applying a selection opens the device and remembers it', async () => {
    mocked.audioSetDevice.mockResolvedValue(opened);
    const a = new AudioStore();
    await a.applySelection(sel);
    expect(a.selection).toEqual(sel);
    expect(a.opened).toEqual(opened);
    expect(loadSelection()).toEqual(sel);
  });

  it('a device that will not open keeps the old selection and reports why', async () => {
    mocked.audioSetDevice.mockRejectedValue('device busy');
    const a = new AudioStore();
    a.selection = { ...defaultSelection(), id: 'wasapi:ok' };
    await expect(a.applySelection(sel)).rejects.toBe('device busy');
    expect(a.selection.id).toBe('wasapi:ok');
    expect(a.error).toBe('device busy');
    expect(loadSelection()).toEqual(defaultSelection());
  });

  it('restore reopens the saved device', async () => {
    localStorage.setItem('audioDevice.v1', JSON.stringify(sel));
    mocked.audioDevices.mockResolvedValue([]);
    mocked.audioSetDevice.mockResolvedValue(opened);
    const a = new AudioStore();
    await a.restore();
    expect(mocked.audioSetDevice).toHaveBeenCalledWith(sel);
    expect(a.selection).toEqual(sel);
  });

  it('restore falls back to the default device when the saved one is gone', async () => {
    localStorage.setItem('audioDevice.v1', JSON.stringify(sel));
    mocked.audioDevices.mockResolvedValue([]);
    mocked.audioSetDevice.mockRejectedValue('not found');
    const a = new AudioStore();
    await a.restore();
    expect(a.selection).toEqual(defaultSelection());
  });

  it('restore with nothing saved does not touch the device', async () => {
    mocked.audioDevices.mockResolvedValue([]);
    const a = new AudioStore();
    await a.restore();
    expect(mocked.audioSetDevice).not.toHaveBeenCalled();
  });

  it('lists devices from the engine', async () => {
    const d = { id: 'wasapi:x', host: 'wasapi', name: 'Speakers', channels: 2, default_rate: 48000, is_default: true };
    mocked.audioDevices.mockResolvedValue([d]);
    const a = new AudioStore();
    await a.refreshDevices();
    expect(a.devices).toEqual([d]);
  });
});
