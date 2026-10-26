// @vitest-environment jsdom
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { emptyPlayback } from './api';
import AudioSettings from './AudioSettings.svelte';
import { audio, defaultSelection } from './audio.svelte';
import { installBackend, SAMPLE_DEVICES, type Backend } from '../test/backend';

let backend: Backend;
const user = () => userEvent.setup();

async function mount(selection = defaultSelection(), cfg: Parameters<typeof installBackend>[1] = {}) {
  backend = installBackend(h.invoke, cfg);
  localStorage.clear();
  audio.stop();
  audio.error = null;
  audio.opened = null;
  audio.state = emptyPlayback();
  audio.selection = selection;
  audio.devices = [...SAMPLE_DEVICES];
  const onclose = vi.fn();
  render(AudioSettings, { onclose });
  await waitFor(() => expect(backend.of('audio_devices')).toHaveLength(1)); // it rescans on open
  return { onclose };
}

beforeEach(() => {
  h.invoke.mockReset();
});

describe('audio output settings', () => {
  it('lists devices grouped by audio API with ASIO first', async () => {
    await mount();
    const headings = [...document.querySelectorAll('.group')].map((g) => g.textContent);
    expect(headings[0]).toMatch(/^ASIO/);
    expect(headings[1]).toMatch(/WASAPI/);
    expect(screen.getByRole('radio', { name: /ASIO4ALL v2/ })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /Speakers/ })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /System default output/ })).toBeChecked();
  });

  it('marks the current selection', async () => {
    await mount({ id: 'asio:DJM-S7 ASIO', out_pair: 0, buffer_frames: null });
    expect(screen.getByRole('radio', { name: /DJM-S7 ASIO/ })).toBeChecked();
  });

  it('learns an ASIO device channel count on demand and offers its output pairs', async () => {
    const u = user();
    await mount();
    expect(screen.getByLabelText('Output channels')).toBeDisabled(); // the system default is stereo
    await u.click(screen.getByRole('radio', { name: /DJM-S7 ASIO/ }));
    await waitFor(() => expect(backend.of('audio_device_details')).toEqual([{ id: 'asio:DJM-S7 ASIO' }]));
    const select = screen.getByLabelText('Output channels');
    await waitFor(() => expect(within(select).getAllByRole('option')).toHaveLength(4));
    expect(within(select).getAllByRole('option').map((o) => o.textContent)).toEqual(['Outputs 1-2', 'Outputs 3-4', 'Outputs 5-6', 'Outputs 7-8']);
  });

  it('does not re-query a device whose channel count is already known', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('radio', { name: /Speakers/ })); // listed with 2 channels already
    expect(backend.of('audio_device_details')).toHaveLength(0);
  });

  it('applies the chosen device, output pair and buffer size, then closes', async () => {
    const u = user();
    const { onclose } = await mount();
    await u.click(screen.getByRole('radio', { name: /DJM-S7 ASIO/ }));
    await waitFor(() => expect(within(screen.getByLabelText('Output channels')).getAllByRole('option')).toHaveLength(4));
    await u.selectOptions(screen.getByLabelText('Output channels'), '1');
    await u.selectOptions(screen.getByLabelText('Buffer size'), '256');
    await u.click(screen.getByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(backend.of('audio_set_device')).toEqual([{ selection: { id: 'asio:DJM-S7 ASIO', out_pair: 1, buffer_frames: 256 } }]);
    expect(JSON.parse(localStorage.getItem('audioDevice.v1')!)).toEqual({ id: 'asio:DJM-S7 ASIO', out_pair: 1, buffer_frames: 256 });
    expect(audio.opened?.out_pair).toBe(1);
  });

  it('shows the reason and stays open when a device will not open', async () => {
    const u = user();
    const { onclose } = await mount();
    backend.fail('audio_set_device', 'could not open DJM-S7 ASIO: device is in use by another program');
    await u.click(screen.getByRole('radio', { name: /DJM-S7 ASIO/ }));
    await u.click(screen.getByRole('button', { name: 'Apply' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('device is in use by another program');
    expect(onclose).not.toHaveBeenCalled();
    expect(localStorage.getItem('audioDevice.v1')).toBeNull(); // a failed choice is not remembered
    // ...and it can be retried once the device is free.
    backend.fail('audio_set_device', null);
    await u.click(screen.getByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(onclose).toHaveBeenCalled());
  });

  it('Rescan asks the engine for the devices again', async () => {
    const u = user();
    await mount();
    // While the initial scan is running the button reads "Scanning…"; wait for it to settle.
    await u.click(await screen.findByRole('button', { name: 'Rescan' }));
    await waitFor(() => expect(backend.of('audio_devices')).toHaveLength(2));
  });

  it('Reset goes back to the system default', async () => {
    const u = user();
    await mount({ id: 'asio:ASIO4ALL v2', out_pair: 2, buffer_frames: 512 });
    await u.click(screen.getByRole('button', { name: 'Reset' }));
    expect(screen.getByRole('radio', { name: /System default output/ })).toBeChecked();
  });

  it('Cancel, Escape and a click outside all close without changing anything', async () => {
    const u = user();
    const { onclose } = await mount();
    await u.click(screen.getByRole('button', { name: 'Cancel' }));
    await u.keyboard('{Escape}');
    await u.click(document.querySelector('.backdrop')!);
    expect(onclose).toHaveBeenCalledTimes(3);
    expect(backend.of('audio_set_device')).toHaveLength(0);
  });

  it('explains that ASIO is usually exclusive', async () => {
    await mount();
    expect(screen.getByText(/close Serato/)).toBeInTheDocument();
  });

  it('copes with no devices at all', async () => {
    await mount(defaultSelection(), { devices: [] });
    audio.devices = [];
    expect(await screen.findByText(/No output devices were found/)).toBeInTheDocument();
  });

  it('buffer sizes show their latency at the open sample rate', async () => {
    await mount();
    const text = within(screen.getByLabelText('Buffer size')).getAllByRole('option').map((o) => o.textContent);
    expect(text).toContain('Driver default');
    expect(text).toContain('256 samples · 5.8 ms'); // 256 / 44100
  });
});
