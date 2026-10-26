// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: h.open, save: vi.fn() }));

import { audio } from './audio.svelte';
import Editor, { EDITOR_LABEL, type ExportRequest } from './Editor.svelte';
import { entry, installBackend, SAMPLE_TOOL, type Backend } from '../test/backend';

const one = entry({ name: 'Serato Recording 1.aif', duration_secs: 3600, recorded: '2024-08-03 21:58:12' });
const two = entry({ name: 'Serato Recording 2.aif', duration_secs: 1800, recorded: '2025-10-04 21:11:51' });

let backend: Backend;
const user = () => userEvent.setup();

function mount(entries = [two, one], tool = SAMPLE_TOOL, durationOverride?: number) {
  backend = installBackend(h.invoke);
  backend.durationOverride = durationOverride;
  audio.stop();
  audio.state = { ...audio.state, loaded: false, playing: false, position_secs: 0, duration_secs: 0 };
  const onstart = vi.fn<(r: ExportRequest) => void>();
  const onclose = vi.fn();
  const view = render(Editor, { entries, tool, onclose, onstart, onlocate: vi.fn(), onsettings: vi.fn() });
  return { ...view, onstart, onclose };
}

/** Pretend the engine reports a playhead position (as the real engine would after a seek). */
function playheadAt(secs: number, duration = 5400) {
  audio.label = EDITOR_LABEL;
  audio.state = { ...audio.state, loaded: true, position_secs: secs, duration_secs: duration };
}

const clipRows = () => screen.getByLabelText('Clips').querySelectorAll('.rows .clip');
const startInput = (n: number) => screen.getByLabelText(`Start of clip ${n}`) as HTMLInputElement;
const endInput = (n: number) => screen.getByLabelText(`End of clip ${n}`) as HTMLInputElement;

beforeEach(() => {
  localStorage.clear();
  h.invoke.mockReset();
  h.open.mockReset();
});

describe('opening', () => {
  it('orders the clips by recording time, earliest first, regardless of selection order', () => {
    mount([two, one]);
    const names = [...clipRows()].map((r) => r.querySelector('.nm')!.textContent);
    expect(names).toEqual(['Serato Recording 1.aif', 'Serato Recording 2.aif']);
  });

  it('shows the whole recording as the default start and end', () => {
    mount();
    expect(startInput(1).value).toBe('0:00.0');
    expect(endInput(1).value).toBe('1:00:00.0');
    expect(endInput(2).value).toBe('30:00.0');
  });

  it('titles itself for combining or editing', () => {
    mount([one, two]);
    expect(screen.getByRole('heading', { name: 'Combine & export' })).toBeInTheDocument();
  });

  it('single file: edit title and the file name defaults to the recording name', () => {
    mount([one]);
    expect(screen.getByRole('heading', { name: 'Edit & export' })).toBeInTheDocument();
    expect(screen.getByText('Serato Recording 1.aif', { selector: '.summary' })).toBeInTheDocument;
  });

  it('loads the clips into the playback engine (debounced) so Play works at once', async () => {
    mount();
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    const load = backend.of('audio_load')[0] as { clips: { path: string }[]; startSecs: number };
    expect(load.clips.map((c) => c.path)).toEqual(['C:/rec/Serato Recording 1.aif', 'C:/rec/Serato Recording 2.aif']);
    expect(load.startSecs).toBe(0);
    expect(audio.label).toBe(EDITOR_LABEL);
  });

  it('pauses playback when closed', async () => {
    const { unmount } = mount();
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    unmount();
    await waitFor(() => expect(backend.of('audio_pause')).toHaveLength(1));
  });

  it('Escape closes it', async () => {
    const { onclose } = mount();
    await user().keyboard('{Escape}');
    expect(onclose).toHaveBeenCalled();
  });
});

describe('editing', () => {
  it('splitting at the playhead cuts the clip in two and Remove drops one half', async () => {
    const u = user();
    mount([one]);
    playheadAt(1200, 3600);
    await u.click(screen.getByRole('button', { name: 'Split at playhead' }));
    expect(clipRows()).toHaveLength(2);
    expect(endInput(1).value).toBe('20:00.0');
    expect(startInput(2).value).toBe('20:00.0');
    expect(endInput(2).value).toBe('1:00:00.0');
    // Cut out the second half: select it and remove it.
    await u.click(clipRows()[1]);
    await u.click(screen.getByRole('button', { name: 'Remove clip' }));
    expect(clipRows()).toHaveLength(1);
  });

  it('cannot split at the very start or end of a clip', async () => {
    mount([one]);
    playheadAt(0, 3600);
    expect(screen.getByRole('button', { name: 'Split at playhead' })).toBeDisabled();
    playheadAt(3600, 3600);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Split at playhead' })).toBeDisabled());
  });

  it('trim start/end here cut to the playhead', async () => {
    const u = user();
    mount([one]);
    playheadAt(600, 3600);
    await u.click(screen.getByRole('button', { name: 'Trim start here' }));
    expect(startInput(1).value).toBe('10:00.0');
    playheadAt(2400, 3000);
    await u.click(screen.getByRole('button', { name: 'Trim end here' }));
    // The playhead is on the trimmed timeline: 2400 s in is 600 + 2400 = 3000 s (50:00) of the source.
    expect(endInput(1).value).toBe('50:00.0');
  });

  it('typed start and end times trim the clip, and nonsense is rejected', async () => {
    const u = user();
    mount([one]);
    const start = startInput(1);
    await u.clear(start);
    await u.type(start, '1:30');
    await u.tab();
    expect(start.value).toBe('1:30.0');
    const end = endInput(1);
    await u.clear(end);
    await u.type(end, 'not a time');
    await u.tab();
    expect(end.value).toBe('1:00:00.0'); // reverted
  });

  it('the trim cannot cross itself', async () => {
    const u = user();
    mount([one]);
    const end = endInput(1);
    await u.clear(end);
    await u.type(end, '0');
    await u.tab();
    // Clamped to one frame past the start rather than collapsing to nothing or going negative.
    await waitFor(() => {
      const last = backend.of('audio_load').at(-1) as { clips: { start_frame: number; end_frame: number | null }[] } | undefined;
      expect(last?.clips[0]).toMatchObject({ start_frame: 0, end_frame: 1 });
    }, { timeout: 2000 });
  });

  it('reset trim restores the whole recording', async () => {
    const u = user();
    mount([one]);
    const start = startInput(1);
    await u.clear(start);
    await u.type(start, '5:00');
    await u.tab();
    await u.click(screen.getByRole('button', { name: 'Reset trim' }));
    expect(startInput(1).value).toBe('0:00.0');
  });

  it('moves a clip earlier and later', async () => {
    const u = user();
    mount([one, two]);
    await u.click(clipRows()[1]);
    await u.click(screen.getByRole('button', { name: 'Move clip earlier' }));
    const names = [...clipRows()].map((r) => r.querySelector('.nm')!.textContent);
    expect(names).toEqual(['Serato Recording 2.aif', 'Serato Recording 1.aif']);
    expect(screen.getByRole('button', { name: 'Move clip earlier' })).toBeDisabled(); // now first
  });

  describe('reordering from the clip list', () => {
    const names = () => [...clipRows()].map((r) => r.querySelector('.nm')!.textContent);
    // jsdom has no layout: give each row a 30px slot so the pointer position means something.
    const layout = () =>
      clipRows().forEach((r, i) => {
        r.getBoundingClientRect = () => ({ top: i * 30, height: 30, bottom: i * 30 + 30, left: 0, right: 100, width: 100, x: 0, y: i * 30, toJSON() {} }) as DOMRect;
      });

    it('dragging a row grip below the next row swaps them', async () => {
      mount([one, two]);
      layout();
      const grip = screen.getByLabelText(/Reorder clip 1/);
      await fireEvent.pointerDown(grip, { button: 0, pointerId: 1, clientY: 10 });
      await fireEvent.pointerMove(grip, { pointerId: 1, clientY: 50 });
      expect(clipRows()[1]).toHaveClass('drop-after');
      await fireEvent.pointerUp(grip, { pointerId: 1, clientY: 50 });
      expect(names()).toEqual(['Serato Recording 2.aif', 'Serato Recording 1.aif']);
    });

    it('dragging up above the first row moves it to the top', async () => {
      mount([one, two]);
      layout();
      const grip = screen.getByLabelText(/Reorder clip 2/);
      await fireEvent.pointerDown(grip, { button: 0, pointerId: 1, clientY: 40 });
      await fireEvent.pointerMove(grip, { pointerId: 1, clientY: -5 });
      await fireEvent.pointerUp(grip, { pointerId: 1, clientY: -5 });
      expect(names()).toEqual(['Serato Recording 2.aif', 'Serato Recording 1.aif']);
    });

    it('releasing where it started changes nothing', async () => {
      mount([one, two]);
      layout();
      const grip = screen.getByLabelText(/Reorder clip 1/);
      await fireEvent.pointerDown(grip, { button: 0, pointerId: 1, clientY: 10 });
      await fireEvent.pointerUp(grip, { pointerId: 1, clientY: 12 });
      expect(names()).toEqual(['Serato Recording 1.aif', 'Serato Recording 2.aif']);
    });

    it('the keyboard moves a focused grip with Up and Down', async () => {
      mount([one, two]);
      await fireEvent.keyDown(screen.getByLabelText(/Reorder clip 1/), { key: 'ArrowDown' });
      expect(names()).toEqual(['Serato Recording 2.aif', 'Serato Recording 1.aif']);
    });
  });

  it('will not remove the only clip', () => {
    mount([one]);
    expect(screen.getByRole('button', { name: 'Remove clip' })).toBeDisabled();
  });

  it('keyboard: S splits, [ trims, Delete removes', async () => {
    const u = user();
    mount([one]);
    playheadAt(1800, 3600);
    await u.keyboard('s');
    expect(clipRows()).toHaveLength(2);
    await u.keyboard('{Delete}');
    expect(clipRows()).toHaveLength(1);
  });

  it('Space toggles playback and the arrows move the playhead', async () => {
    const u = user();
    mount([one]);
    playheadAt(100, 3600);
    await waitFor(() => expect(audio.state.loaded).toBe(true));
    await u.keyboard(' ');
    await waitFor(() => expect(backend.of('audio_play')).toHaveLength(1));
    await u.keyboard('{ArrowRight}');
    await waitFor(() => expect(backend.of('audio_seek').at(-1)).toEqual({ secs: 101 }));
    await u.keyboard('{Shift>}{ArrowLeft}{/Shift}');
    await waitFor(() => expect(backend.of('audio_seek').at(-1)).toEqual({ secs: 91 }));
  });

  it('Ctrl+arrow moves the playhead by exactly one sample frame and the position is shown', async () => {
    const u = user();
    mount([one]);
    playheadAt(100, 3600);
    await waitFor(() => expect(audio.state.loaded).toBe(true));
    await u.keyboard('{Control>}{ArrowRight}{/Control}');
    await waitFor(() => expect(backend.of('audio_seek').length).toBeGreaterThan(0));
    const secs = (backend.of('audio_seek').at(-1) as { secs: number }).secs;
    expect(secs * 44100).toBeCloseTo(100 * 44100 + 1, 5);
    expect(screen.getByTitle(/Ctrl\+Left \/ Right moves one sample/)).toHaveTextContent(/clip 1 · sample 4,410,001/);
  });

  it('start and end accept an exact sample frame as #N', async () => {
    const u = user();
    mount([one]);
    const start = startInput(1);
    await u.clear(start);
    await u.type(start, '#44101');
    await u.tab();
    expect(start.value).toBe('0:01.0'); // 1.00002 s, shown to a tenth
    expect(start.title).toMatch(/Sample frame 44,101/);
    const end = endInput(1);
    await u.clear(end);
    await u.type(end, '#88200');
    await u.tab();
    expect(end.title).toMatch(/Sample frame 88,200/);
  });

  it('edits are sent to the engine again, from where the playhead is', async () => {
    const u = user();
    mount([one]);
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    playheadAt(900, 3600);
    await u.click(screen.getByRole('button', { name: 'Trim start here' }));
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(2), { timeout: 2000 });
    const second = backend.of('audio_load')[1] as { clips: { start_frame: number }[]; startSecs: number };
    expect(second.clips[0].start_frame).toBe(900 * 44100);
    expect(second.startSecs).toBeCloseTo(900, 0);
  });
});

describe('playing from the timeline', () => {
  const canvas = () => screen.getByTestId('timeline') as HTMLCanvasElement;
  const press = async (x: number, y: number) => {
    await fireEvent(canvas(), new PointerEvent('pointerdown', { clientX: x, clientY: y, button: 0, pointerId: 1, bubbles: true }));
    await fireEvent(canvas(), new PointerEvent('pointerup', { clientX: x, clientY: y, button: 0, pointerId: 1, bubbles: true }));
  };

  it('clicking the waveform moves the playhead there and starts playing', async () => {
    mount([one], SAMPLE_TOOL, 3600); // one hour, fitted to the 1000 px view
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    await press(500, 80);
    await waitFor(() => expect(backend.of('audio_play')).toHaveLength(1));
    const seek = backend.of('audio_seek').at(-1) as { secs: number };
    expect(seek.secs).toBeCloseTo(1800, -1); // the middle of the hour
    expect(audio.state.playing).toBe(true);
  });

  it('clicking the ruler does the same', async () => {
    mount([one], SAMPLE_TOOL, 3600);
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    await press(250, 10);
    await waitFor(() => expect(backend.of('audio_play')).toHaveLength(1));
    expect((backend.of('audio_seek').at(-1) as { secs: number }).secs).toBeCloseTo(900, -1);
  });

  it('clicking again while playing jumps without stopping', async () => {
    mount([one], SAMPLE_TOOL, 3600);
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    await press(200, 80);
    await press(700, 80);
    await waitFor(() => expect(backend.of('audio_seek')).toHaveLength(2));
    expect(backend.of('audio_pause')).toHaveLength(0);
    expect(audio.state.playing).toBe(true);
  });

  it('reordering by the name bar does not start playback', async () => {
    mount([one, two]);
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    await fireEvent(canvas(), new PointerEvent('pointerdown', { clientX: 100, clientY: 34, button: 0, pointerId: 1, bubbles: true }));
    await fireEvent(canvas(), new PointerEvent('pointermove', { clientX: 900, clientY: 34, button: 0, pointerId: 1, bubbles: true }));
    await fireEvent(canvas(), new PointerEvent('pointerup', { clientX: 900, clientY: 34, button: 0, pointerId: 1, bubbles: true }));
    expect(backend.of('audio_play')).toHaveLength(0);
    const names = [...clipRows()].map((r) => r.querySelector('.nm')!.textContent);
    expect(names).toEqual(['Serato Recording 2.aif', 'Serato Recording 1.aif']);
  });

  it('dragging across the waveform to zoom does not start playback or change the clips', async () => {
    mount([one]);
    await waitFor(() => expect(backend.of('audio_load')).toHaveLength(1), { timeout: 2000 });
    await fireEvent(canvas(), new PointerEvent('pointerdown', { clientX: 200, clientY: 80, button: 0, pointerId: 1, bubbles: true }));
    await fireEvent(canvas(), new PointerEvent('pointermove', { clientX: 600, clientY: 80, button: 0, pointerId: 1, bubbles: true }));
    await fireEvent(canvas(), new PointerEvent('pointerup', { clientX: 600, clientY: 80, button: 0, pointerId: 1, bubbles: true }));
    expect(backend.of('audio_play')).toHaveLength(0);
    expect(endInput(1).value).toBe('1:00:00.0');
  });
});

describe('zoom controls', () => {
  it('has labelled zoom buttons and +, -, 0 keys that do not fire inside text fields', async () => {
    const u = user();
    mount([one, two]);
    expect(screen.getByText('Zoom')).toBeInTheDocument();
    const zin = screen.getByRole('button', { name: 'Zoom in' });
    const zout = screen.getByRole('button', { name: 'Zoom out' });
    await u.click(zin);
    await u.click(zout);
    await u.keyboard('+-0'); // none of these may throw or edit a clip
    expect(clipRows()).toHaveLength(2);
    await u.click(startInput(1));
    await u.keyboard('-');
    expect(clipRows()).toHaveLength(2);
  });
});

describe('exporting', () => {
  it('combines into one file by default and describes the result', async () => {
    mount([one, two]);
    expect(await screen.findByText(/44100 Hz, 16-bit, 2 ch/)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /Combine into one file/ })).toBeChecked();
  });

  it('exports AIFF with the default name and no tags', async () => {
    const u = user();
    const { onstart } = mount([one, two]);
    await u.click(screen.getByRole('button', { name: 'Export' }));
    const req = onstart.mock.calls[0][0];
    expect(req.target).toEqual({ kind: 'aiff' });
    expect(req.outputDir).toBe('C:/rec');
    expect(req.jobs).toHaveLength(1);
    expect(req.jobs[0].stem).toBe('Serato Recording 1 (combined)');
    expect(req.jobs[0].clips.map((c) => c.path)).toHaveLength(2);
    expect(Object.values(req.jobs[0].metadata).every((v) => v === '')).toBe(true);
  });

  it('exports MP3 with the chosen mode, bitrate and ID3 options, and expands the tag templates', async () => {
    const u = user();
    const { onstart } = mount([one]);
    await u.click(screen.getByRole('radio', { name: /MP3/ }));
    await u.selectOptions(screen.getByLabelText('Bitrate mode'), 'vbr');
    await u.selectOptions(screen.getByLabelText('Quality'), '0');
    await u.selectOptions(screen.getByLabelText('ID3 tag version'), '4');
    await u.click(screen.getByRole('checkbox', { name: /also write ID3v1/ }));
    await u.click(screen.getByRole('tab', { name: /Name, folder/ }));
    await u.type(screen.getByLabelText('Artist'), 'DJ Test');
    await u.type(screen.getByLabelText('Genre'), 'House');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    const req = onstart.mock.calls[0][0];
    expect(req.target).toEqual({ kind: 'mp3', mode: 'vbr', value: 0, id3_version: 4, id3v1: true });
    expect(req.jobs[0].stem).toBe('Serato Recording 1');
    expect(req.jobs[0].metadata).toMatchObject({
      title: 'Serato Recording 1',
      artist: 'DJ Test',
      genre: 'House',
      date: '2024-08-03',
      comment: 'Recorded 2024-08-03 21:58:12',
    });
  });

  it('exports AAC with the Apple encoder by default and a CBR bitrate', async () => {
    const u = user();
    const { onstart } = mount([one]);
    await u.click(screen.getByRole('radio', { name: /AAC/ }));
    expect(screen.getByLabelText('Encoder')).toHaveValue('apple_coreaudio');
    await u.selectOptions(screen.getByLabelText('Bitrate'), '320');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    expect(onstart.mock.calls[0][0].target).toEqual({ kind: 'aac', encoder: 'apple_coreaudio', mode: 'cbr', value: 320 });
  });

  it('lets you pick another AAC encoder and a VBR quality', async () => {
    const u = user();
    const { onstart } = mount([one]);
    await u.click(screen.getByRole('radio', { name: /AAC/ }));
    await u.selectOptions(screen.getByLabelText('Encoder'), 'aac');
    await u.selectOptions(screen.getByLabelText('Bitrate mode'), 'vbr');
    await u.selectOptions(screen.getByLabelText('Quality'), '5');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    expect(onstart.mock.calls[0][0].target).toEqual({ kind: 'aac', encoder: 'aac', mode: 'vbr', value: 5 });
  });

  it('one file per clip makes a job each, numbered by the template', async () => {
    const u = user();
    const { onstart } = mount([one, two]);
    await u.click(screen.getByRole('radio', { name: /One file per clip/ }));
    await u.click(screen.getByRole('tab', { name: /Name, folder/ }));
    const name = screen.getByLabelText('File name template');
    await u.clear(name);
    await u.type(name, '{{n2} - {{name}');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    const req = onstart.mock.calls[0][0];
    expect(req.jobs.map((j) => j.stem)).toEqual(['01 - Serato Recording 1', '02 - Serato Recording 2']);
    expect(req.jobs.every((j) => j.clips.length === 1)).toBe(true);
  });

  it('an unknown placeholder blocks the export and says which', async () => {
    const u = user();
    mount([one]);
    await u.click(screen.getByRole('tab', { name: /Name, folder/ }));
    const name = screen.getByLabelText('File name template');
    await u.clear(name);
    await u.type(name, '{{nope}');
    expect(screen.getAllByText(/Unknown placeholder/).length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled();
  });

  it('an empty file name blocks the export', async () => {
    const u = user();
    mount([one]);
    await u.click(screen.getByRole('tab', { name: /Name, folder/ }));
    await u.clear(screen.getByLabelText('File name template'));
    expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled();
  });

  it('a clip-format mismatch reported by the backend blocks combining and says why', async () => {
    backend = installBackend(h.invoke, { failures: { plan_info: 'clip 2 is 48000 Hz / 16-bit / 2 ch, but the first clip is 44100 Hz / 16-bit / 2 ch' } });
    audio.stop();
    render(Editor, { entries: [one, two], tool: SAMPLE_TOOL, onclose: vi.fn(), onstart: vi.fn(), onlocate: vi.fn(), onsettings: vi.fn() });
    expect(await screen.findByText(/clip 2 is 48000 Hz/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled();
    // One file per clip does not need matching formats, so it is still possible.
    await userEvent.setup().click(screen.getByRole('radio', { name: /One file per clip/ }));
    expect(screen.getByRole('button', { name: 'Export' })).toBeEnabled();
  });
  it('warns when AIFF output would pass 4 GB and offers a way out', async () => {
    h.invoke.mockReset();
    backend = installBackend(h.invoke);
    const orig = h.invoke.getMockImplementation()!;
    h.invoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) =>
      cmd === 'plan_info' ? { total_frames: 1, duration_secs: 9000, bytes: 5_000_000_000, format: 'x', fits_aiff: false } : orig(cmd, args),
    );
    audio.stop();
    render(Editor, { entries: [one, two], tool: SAMPLE_TOOL, onclose: vi.fn(), onstart: vi.fn(), onlocate: vi.fn(), onsettings: vi.fn() });
    expect(await screen.findByText(/Over AIFF's 4 GB limit/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled();
    await userEvent.setup().click(screen.getByRole('radio', { name: /MP3/ }));
    expect(screen.getByRole('button', { name: 'Export' })).toBeEnabled();
  });

  it('MP3 and AAC are unavailable without ffmpeg', () => {
    mount([one], { ffmpeg_path: null, ffmpeg_version: null, aac_encoders: [], best_aac: null, mp3: false, apple: null });
    expect(screen.getByRole('radio', { name: /MP3/ })).toBeDisabled();
    expect(screen.getByRole('radio', { name: /AAC/ })).toBeDisabled();
    expect(screen.getByText(/ffmpeg was not found/)).toBeInTheDocument();
  });

  it('remembers the chosen settings for next time', async () => {
    const u = user();
    mount([one]);
    await u.click(screen.getByRole('radio', { name: /MP3/ }));
    await u.selectOptions(screen.getByLabelText('Bitrate'), '192');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    const saved = JSON.parse(localStorage.getItem('exportSettings.v1')!);
    expect(saved.format).toMatchObject({ kind: 'mp3', mp3Mode: 'cbr', mp3Value: 192 });
  });

  it('starts from remembered settings', async () => {
    localStorage.setItem(
      'exportSettings.v1',
      JSON.stringify({ format: { kind: 'mp3', mp3Mode: 'abr', mp3Value: 256, id3Version: 3, id3v1: false, aacEncoder: 'aac', aacMode: 'cbr', aacValue: 256 }, nameTemplate: '{date} - {name}' }),
    );
    const { onstart } = mount([one]);
    await userEvent.setup().click(screen.getByRole('button', { name: 'Export' }));
    expect(onstart.mock.calls[0][0].target).toMatchObject({ kind: 'mp3', mode: 'abr', value: 256 });
    expect(onstart.mock.calls[0][0].jobs[0].stem).toBe('2024-08-03 - Serato Recording 1');
  });

  it('pauses playback when the export starts', async () => {
    const u = user();
    mount([one]);
    await u.click(screen.getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('audio_pause').length).toBeGreaterThan(0));
  });

  it('has no fades unless asked', async () => {
    const u = user();
    const { onstart } = mount([one]);
    await u.click(screen.getByRole('button', { name: 'Export' }));
    expect(onstart.mock.calls[0][0].jobs[0]).toMatchObject({ fadeInSecs: 0, fadeOutSecs: 0 });
  });

  it('sends the chosen fade in and fade out with the export, whatever the format', async () => {
    const u = user();
    for (const format of [/AIFF/, /MP3/, /AAC/]) {
      const { onstart, unmount } = mount([one]);
      await u.click(screen.getByRole('radio', { name: format }));
      await u.click(screen.getByRole('tab', { name: /Fades/ }));
      await u.type(screen.getByLabelText('Fade in (seconds)'), '2');
      await u.type(screen.getByLabelText('Fade out (seconds)'), '4.5');
      await u.click(screen.getByRole('button', { name: 'Export' }));
      expect(onstart.mock.calls[0][0].jobs[0], String(format)).toMatchObject({ fadeInSecs: 2, fadeOutSecs: 4.5 });
      unmount();
      localStorage.clear();
    }
  });

  it('turns nonsense fade values into no fade and caps long ones', async () => {
    const u = user();
    const { onstart } = mount([one]);
    await u.click(screen.getByRole('tab', { name: /Fades/ }));
    const fin = screen.getByLabelText('Fade in (seconds)');
    const fout = screen.getByLabelText('Fade out (seconds)');
    await u.clear(fin);
    await u.type(fin, '-3');
    await u.clear(fout);
    await u.type(fout, '900');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    expect(onstart.mock.calls[0][0].jobs[0]).toMatchObject({ fadeInSecs: 0, fadeOutSecs: 60 });
  });

  it('an emptied fade field means no fade', async () => {
    const u = user();
    const { onstart } = mount([one]);
    await u.click(screen.getByRole('tab', { name: /Fades/ }));
    const fin = screen.getByLabelText('Fade in (seconds)');
    await u.type(fin, '3');
    await u.clear(fin);
    await u.click(screen.getByRole('button', { name: 'Export' }));
    expect(onstart.mock.calls[0][0].jobs[0].fadeInSecs).toBe(0);
  });

  it('with one file per clip, every file gets its own fades', async () => {
    const u = user();
    const { onstart } = mount([one, two]);
    await u.click(screen.getByRole('tab', { name: /Fades/ }));
    await u.type(screen.getByLabelText('Fade in (seconds)'), '1');
    await u.type(screen.getByLabelText('Fade out (seconds)'), '1');
    await u.click(screen.getByRole('radio', { name: /One file per clip/ }));
    await u.click(screen.getByRole('button', { name: 'Export' }));
    const jobs = onstart.mock.calls[0][0].jobs;
    expect(jobs).toHaveLength(2);
    expect(jobs.every((j) => j.fadeInSecs === 1 && j.fadeOutSecs === 1)).toBe(true);
  });

  it('remembers the fades for next time', async () => {
    const u = user();
    mount([one]);
    await u.click(screen.getByRole('tab', { name: /Fades/ }));
    await u.type(screen.getByLabelText('Fade in (seconds)'), '3');
    await u.type(screen.getByLabelText('Fade out (seconds)'), '6');
    await u.click(screen.getByRole('button', { name: 'Export' }));
    expect(JSON.parse(localStorage.getItem('exportSettings.v1')!)).toMatchObject({ fadeIn: 3, fadeOut: 6 });
    mount([one]);
    await u.click(screen.getAllByRole('tab', { name: /Fades/ }).at(-1)!);
    expect((screen.getAllByLabelText('Fade in (seconds)').at(-1) as HTMLInputElement).value).toBe('3');
    expect((screen.getAllByLabelText('Fade out (seconds)').at(-1) as HTMLInputElement).value).toBe('6');
  });
  it('shows an estimated size for lossy formats', async () => {
    const u = user();
    mount([one]);
    await u.click(screen.getByRole('radio', { name: /MP3/ }));
    expect(screen.getByText(/Estimated size: about/)).toBeInTheDocument();
    const text = screen.getByText(/Estimated size/).textContent!;
    expect(text).toMatch(/\d+ MB/); // 1 h at 320 kbps is about 144 MB
    expect(within(document.body).getByText(/Estimated size/)).toBeInTheDocument();
  });
});
