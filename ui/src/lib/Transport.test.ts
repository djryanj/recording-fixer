// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { emptyPlayback } from './api';
import { audio } from './audio.svelte';
import Transport from './Transport.svelte';
import { installBackend, type Backend } from '../test/backend';

let backend: Backend;

function setup(over: Partial<ReturnType<typeof emptyPlayback>> = {}, props: { compact?: boolean } = {}) {
  backend = installBackend(h.invoke);
  audio.stop();
  audio.error = null;
  audio.opened = null;
  audio.label = '';
  audio.state = { ...emptyPlayback(), ...over };
  Object.assign(backend.playback, audio.state);
  const onsettings = vi.fn();
  render(Transport, { onsettings, ...props });
  return { onsettings };
}

beforeEach(() => {
  h.invoke.mockReset();
});

describe('transport', () => {
  it('says nothing is loaded and disables play', () => {
    setup();
    expect(screen.getByText('Nothing loaded')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Play' })).toBeDisabled();
    expect(screen.getByRole('slider', { name: 'Seek' })).toBeDisabled();
    expect(screen.getByTestId('position')).toHaveTextContent('0:00.0');
  });

  it('shows what is loaded with its position and length', () => {
    audio.label = 'Serato Recording 2.aif';
    setup({ loaded: true, position_secs: 125.46, duration_secs: 4800 });
    audio.label = 'Serato Recording 2.aif';
    expect(screen.getByTestId('position')).toHaveTextContent('2:05.4');
    expect(screen.getByText('/ 1:20:00')).toBeInTheDocument();
  });

  it('play and pause buttons drive the engine', async () => {
    const u = userEvent.setup();
    setup({ loaded: true, duration_secs: 100 });
    await u.click(screen.getByRole('button', { name: 'Play' }));
    await waitFor(() => expect(backend.of('audio_play')).toHaveLength(1));
    expect(await screen.findByRole('button', { name: 'Pause' })).toBeInTheDocument();
    await u.click(screen.getByRole('button', { name: 'Pause' }));
    await waitFor(() => expect(backend.of('audio_pause')).toHaveLength(1));
  });

  it('seeks when the seek bar is released, not on every drag step', async () => {
    setup({ loaded: true, duration_secs: 100 });
    const bar = screen.getByRole('slider', { name: 'Seek' });
    await fireEvent.input(bar, { target: { value: '30' } });
    expect(backend.of('audio_seek')).toHaveLength(0);
    expect(screen.getByTestId('position')).toHaveTextContent('0:30.0'); // follows the drag
    await fireEvent.change(bar, { target: { value: '30' } });
    await waitFor(() => expect(backend.of('audio_seek')).toEqual([{ secs: 30 }]));
  });

  it('hides the seek bar in compact mode (the timeline seeks there)', () => {
    setup({ loaded: true, duration_secs: 100 }, { compact: true });
    expect(screen.queryByRole('slider', { name: 'Seek' })).not.toBeInTheDocument();
    expect(screen.getByRole('slider', { name: 'Volume' })).toBeInTheDocument();
  });

  it('volume changes go to the engine', async () => {
    setup({ loaded: true, duration_secs: 100 });
    await fireEvent.input(screen.getByRole('slider', { name: 'Volume' }), { target: { value: '0.4' } });
    await waitFor(() => expect(backend.of('audio_volume')).toEqual([{ volume: 0.4 }]));
  });

  it('shows the output device that is actually open', () => {
    backend = installBackend(h.invoke);
    audio.stop();
    audio.opened = { id: 'asio:X', host: 'asio', name: 'DJM-S7 ASIO', rate: 44100, channels: 8, out_pair: 1, buffer_frames: 256 };
    audio.state = emptyPlayback();
    render(Transport, { onsettings: vi.fn() });
    expect(screen.getByText('DJM-S7 ASIO')).toBeInTheDocument();
    expect(screen.getByText('ASIO · 44.1 kHz · out 3-4')).toBeInTheDocument();
  });

  it('shows the system default before anything is opened, and opens the settings on click', async () => {
    const { onsettings } = setup();
    expect(screen.getByText('System default')).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole('button', { name: /Output/ }));
    expect(onsettings).toHaveBeenCalled();
  });

  it('surfaces engine errors and dropouts', () => {
    setup({ loaded: true, error: 'could not open Voicemeeter: sample clock cannot be determined' });
    expect(screen.getByRole('alert')).toHaveTextContent('sample clock cannot be determined');
  });

  it('warns about dropouts only while playing', () => {
    setup({ loaded: true, playing: true, underruns: 3 });
    expect(screen.getByText(/Audio dropouts: 3/)).toBeInTheDocument();
  });

  it('does not nag about old dropouts once paused', () => {
    setup({ loaded: true, playing: false, underruns: 3 });
    expect(screen.queryByText(/Audio dropouts/)).not.toBeInTheDocument();
  });

  it('the level meter follows the peaks', () => {
    setup({ loaded: true, playing: true, peak_l: 1, peak_r: 0 });
    const bars = document.querySelectorAll('.meter .bar i');
    expect((bars[0] as HTMLElement).style.width).toBe('100%');
    expect((bars[1] as HTMLElement).style.width).toBe('0%');
  });
});
