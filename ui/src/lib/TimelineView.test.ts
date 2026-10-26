// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import Timeline from './Timeline.svelte';
import Harness from '../test/TimelineHarness.svelte';
import type { TClip } from './timeline';
import { installBackend, type Backend } from '../test/backend';

const RATE = 44100;
const clip = (id: number, secs: number, over: Partial<TClip> = {}): TClip => ({
  id,
  path: `C:/rec/${id}.aif`,
  name: `${id}.aif`,
  rate: RATE,
  total: secs * RATE,
  start: 0,
  end: secs * RATE,
  recorded: null,
  modified_ms: 0,
  ...over,
});

// The test environment gives every element a 1000 px wide box, so the timeline is 1000 px wide
// and "fit" maps `total` seconds onto ~998 px.
const W = 1000;
const FIT = (total: number) => (W - 2) / total;
const BODY_Y = 80; // the waveform, below the 26 px ruler and the 18 px clip title strip
const TITLE_Y = 34; // a clip's name bar, the handle for reordering

let backend: Backend;
const canvas = () => screen.getByTestId('timeline') as HTMLCanvasElement;

function mount(clips: TClip[], over: Partial<{ playheadSecs: number; playing: boolean; selectedId: number | null }> = {}) {
  backend = installBackend(h.invoke);
  const props = {
    clips,
    selectedId: over.selectedId ?? null,
    playheadSecs: over.playheadSecs ?? 0,
    playing: over.playing ?? false,
    pxPerSec: 1,
    onselect: vi.fn(),
    onseek: vi.fn(),
    onchange: vi.fn(),
  };
  const view = render(Timeline, props);
  return { ...view, props };
}

const pointer = (type: string, x: number, y = BODY_Y) => fireEvent(canvas(), new PointerEvent(type, { clientX: x, clientY: y, button: 0, pointerId: 1, bubbles: true }));
const click = async (x: number, y = BODY_Y) => {
  await pointer('pointerdown', x, y);
  await pointer('pointerup', x, y);
};

beforeEach(() => {
  h.invoke.mockReset();
});

describe('timeline interaction', () => {
  it('fits all the clips into the view when it first appears', async () => {
    const { props } = mount([clip(1, 100)]);
    await waitFor(() => expect(canvas()).toBeInTheDocument());
    await click(W / 2);
    await tick();
    // The click lands halfway through, so the seek is to about the middle of the 100 s of audio.
    expect(props.onseek.mock.calls.at(-1)![0]).toBeCloseTo(50, 0);
  });

  it('clicking a clip selects it and moves the playhead there', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    const px = FIT(200);
    await click(150 * px); // 50 s into the second clip
    expect(props.onselect).toHaveBeenCalledWith(2);
    expect(props.onseek.mock.calls.at(-1)![0]).toBeCloseTo(150, 0);
  });

  it('clicking the ruler only moves the playhead', async () => {
    const { props } = mount([clip(1, 100)]);
    await tick();
    await click(W * 0.25, 10);
    expect(props.onseek).toHaveBeenCalledTimes(1);
    expect(props.onselect).not.toHaveBeenCalled();
    expect(props.onseek.mock.calls[0][0]).toBeCloseTo(25, 0);
  });

  it('clicking empty space beyond the last clip deselects', async () => {
    backend = installBackend(h.invoke);
    const props = { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
    const { rerender } = render(Harness, props);
    await tick();
    // Trimming shortens the material but the view keeps its scale (it must not move under a drag),
    // which leaves empty space on the right.
    await rerender({ ...props, clips: [clip(1, 100, { end: 30 * RATE })] });
    await tick();
    await click(800);
    expect(props.onselect).toHaveBeenCalledWith(null);
    expect(props.onseek).not.toHaveBeenCalled();
  });

  it('keeps its scale when the clips change, so a trim drag does not slide under the pointer', async () => {
    backend = installBackend(h.invoke);
    const props = { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
    const { rerender } = render(Harness, props);
    await tick();
    const fitted = Number(screen.getByTestId('px').textContent);
    expect(fitted).toBeCloseTo(FIT(100), 2); // fitted to the measured width, not the 800 px placeholder
    await rerender({ ...props, clips: [clip(1, 100, { end: 50 * RATE })] });
    await tick();
    expect(Number(screen.getByTestId('px').textContent)).toBe(fitted);
  });
  it('seeks are clamped to the material', async () => {
    const { props } = mount([clip(1, 100)]);
    await tick();
    await click(0, 10);
    expect(props.onseek.mock.calls.at(-1)![0]).toBeGreaterThanOrEqual(0);
    await click(W, 10);
    expect(props.onseek.mock.calls.at(-1)![0]).toBeLessThanOrEqual(100);
  });

  it('dragging the right edge of a clip trims its end', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    const edge = 100 * FIT(200);
    await pointer('pointerdown', edge - 2); // just left of the boundary: the first clip's end
    await pointer('pointermove', edge - 2 - 100); // drag 100 px left
    await pointer('pointerup', edge - 2 - 100);
    const next = props.onchange.mock.calls.at(-1)![0] as TClip[];
    expect(next[0].end).toBeLessThan(100 * RATE);
    expect(next[0].end / RATE).toBeCloseTo(100 - 100 / FIT(200), 0); // 100 px is about 20 s
    expect(next[0].start).toBe(0);
    expect(next[1]).toEqual(clip(2, 100)); // the neighbour is untouched
  });

  it('dragging the left edge of a clip trims its start', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    const edge = 100 * FIT(200);
    await pointer('pointerdown', edge + 2); // just right of the boundary: the second clip's start
    await pointer('pointermove', edge + 2 + 50);
    await pointer('pointerup', edge + 2 + 50);
    const next = props.onchange.mock.calls.at(-1)![0] as TClip[];
    expect(next[1].start / RATE).toBeCloseTo(50 / FIT(200), 0);
    expect(next[0]).toEqual(clip(1, 100));
  });

  it('a trim drag can never invert or exceed the source', async () => {
    const { props } = mount([clip(1, 100)]);
    await tick();
    const end = 100 * FIT(100);
    await pointer('pointerdown', end - 2);
    await pointer('pointermove', end - 2 + 500); // far past the end
    let next = props.onchange.mock.calls.at(-1)![0] as TClip[];
    expect(next[0].end).toBe(100 * RATE);
    await pointer('pointermove', -500); // far past the start
    next = props.onchange.mock.calls.at(-1)![0] as TClip[];
    expect(next[0].end).toBeGreaterThan(next[0].start);
    await pointer('pointerup', -500);
  });

  it('dragging a clip by its name bar past the threshold reorders it', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100), clip(3, 100)]);
    await tick();
    const px = FIT(300);
    await pointer('pointerdown', 50 * px, TITLE_Y); // the name bar of clip 1
    await pointer('pointermove', 50 * px + 4, TITLE_Y); // under the threshold: still a click
    expect(props.onchange).not.toHaveBeenCalled();
    await pointer('pointermove', 230 * px, TITLE_Y); // in clip 3, before its centre: lands between 2 and 3
    await pointer('pointerup', 230 * px, TITLE_Y);
    const next = props.onchange.mock.calls.at(-1)![0] as TClip[];
    expect(next.map((c) => c.id)).toEqual([2, 1, 3]);
    expect(props.onseek).not.toHaveBeenCalled(); // a reorder must not start playback
  });

  it('dropping a dragged clip at the far end moves it last', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    const px = FIT(200);
    await pointer('pointerdown', 50 * px, TITLE_Y);
    await pointer('pointermove', 199 * px, TITLE_Y);
    await pointer('pointerup', 199 * px, TITLE_Y);
    expect((props.onchange.mock.calls.at(-1)![0] as TClip[]).map((c) => c.id)).toEqual([2, 1]);
  });

  it('dragging a clip back to where it was changes nothing visible', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    const px = FIT(200);
    await pointer('pointerdown', 50 * px, TITLE_Y);
    await pointer('pointermove', 20 * px, TITLE_Y);
    await pointer('pointerup', 20 * px, TITLE_Y);
    const calls = props.onchange.mock.calls;
    expect(calls.length === 0 || (calls.at(-1)![0] as TClip[]).map((c) => c.id).join() === '1,2').toBe(true);
  });

  it('clicking a name bar without dragging plays from there, like clicking the waveform', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    await click(150 * FIT(200), TITLE_Y);
    expect(props.onselect).toHaveBeenCalledWith(2);
    expect(props.onseek.mock.calls.at(-1)![0]).toBeCloseTo(150, 0);
    expect(props.onchange).not.toHaveBeenCalled();
  });

  it('dragging across a waveform zooms to that stretch instead of reordering or seeking', async () => {
    backend = installBackend(h.invoke);
    const props = { clips: [clip(1, 100), clip(2, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
    render(Harness, props);
    await tick();
    const fitted = Number(screen.getByTestId('px').textContent);
    // Drag from 25% to 50% of the view: 50 s to 100 s of the 200 s.
    await pointer('pointerdown', W * 0.25);
    await pointer('pointermove', W * 0.5);
    await pointer('pointerup', W * 0.5);
    await waitFor(() => expect(Number(screen.getByTestId('px').textContent)).toBeGreaterThan(fitted * 3.5));
    // The stretch (about 50 s) now fills the view, give or take the rubber band's pixel rounding.
    expect(Number(screen.getByTestId('px').textContent)).toBeCloseTo((W - 2) / (0.25 * 200 * (W / (W - 2))), 0);
    expect(props.onchange).not.toHaveBeenCalled();
    expect(props.onseek).not.toHaveBeenCalled(); // it was a drag, not a click
    expect(props.onselect).toHaveBeenCalledWith(1); // pressing on a clip still selects it
  });

  describe('pinch zoom', () => {
    const px = () => Number(screen.getByTestId('px').textContent);
    const touch = (type: string, id: number, x: number) =>
      fireEvent(canvas(), new PointerEvent(type, { clientX: x, clientY: BODY_Y, button: 0, pointerId: id, pointerType: 'touch', bubbles: true }));
    const wheel = (deltaY: number) => fireEvent.wheel(canvas().closest('.scroller')!, { ctrlKey: true, deltaY, clientX: 400 });
    const setup = async () => {
      backend = installBackend(h.invoke);
      const props = { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
      render(Harness, props);
      await tick();
      return props;
    };

    it('a mouse-wheel notch with Ctrl zooms about 1.25x, and a small trackpad pinch much less', async () => {
      await setup();
      const fitted = px();
      await wheel(-100);
      await waitFor(() => expect(px()).toBeCloseTo(fitted * 1.25, 1));
      const before = px();
      await wheel(-4); // a trackpad pinch reports ctrl+wheel with tiny deltas
      await waitFor(() => expect(px()).toBeGreaterThan(before));
      expect(px()).toBeLessThan(before * 1.02);
    });

    it('spreading two fingers zooms in, pinching them together zooms back out', async () => {
      const props = await setup();
      const fitted = px();
      await touch('pointerdown', 1, 400);
      await touch('pointerdown', 2, 600);
      await touch('pointermove', 2, 800); // fingers 200 px apart -> 400 px
      await waitFor(() => expect(px()).toBeCloseTo(fitted * 2, 1));
      await touch('pointermove', 2, 600);
      await waitFor(() => expect(px()).toBeCloseTo(fitted, 1));
      await touch('pointerup', 2, 600);
      await touch('pointerup', 1, 400);
      // A pinch is only a zoom: no seek, no selection, no edit.
      expect(props.onseek).not.toHaveBeenCalled();
      expect(props.onchange).not.toHaveBeenCalled();
    });

    it('lifting one finger after a pinch does not turn the other into a tap', async () => {
      const props = await setup();
      await touch('pointerdown', 1, 400);
      await touch('pointerdown', 2, 600);
      await touch('pointermove', 2, 700);
      await touch('pointerup', 2, 700);
      await touch('pointerup', 1, 400);
      expect(props.onseek).not.toHaveBeenCalled();
    });

    it('a single finger tap still plays from there', async () => {
      const props = await setup();
      await touch('pointerdown', 1, 300);
      await touch('pointerup', 1, 300);
      expect(props.onseek).toHaveBeenCalledTimes(1);
    });
  });

  it('a short drag is a click, not a zoom', async () => {
    backend = installBackend(h.invoke);
    const props = { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
    render(Harness, props);
    await tick();
    const fitted = Number(screen.getByTestId('px').textContent);
    await pointer('pointerdown', 300);
    await pointer('pointermove', 303); // 3 px of wobble
    await pointer('pointerup', 303);
    expect(Number(screen.getByTestId('px').textContent)).toBe(fitted);
    expect(props.onseek).toHaveBeenCalledTimes(1);
  });

  it('the zoom stops at the maximum magnification however small the stretch', async () => {
    backend = installBackend(h.invoke);
    const props = { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
    render(Harness, props);
    await tick();
    await pointer('pointerdown', 500);
    await pointer('pointermove', 503); // 6 px: just over the click threshold, about 0.6 s of 100 s
    await pointer('pointerup', 512);
    await waitFor(() => expect(Number(screen.getByTestId('px').textContent)).toBeLessThanOrEqual(RATE * 40));
  });

  describe('down to single samples', () => {
    const px = () => Number(screen.getByTestId('px').textContent);
    const zoomAllTheWay = async () => {
      const scroller = canvas().closest('.scroller')!;
      for (let i = 0; i < 30; i++) {
        await fireEvent.wheel(scroller, { ctrlKey: true, deltaY: -1000, clientX: 400 });
        await tick();
      }
    };
    const setup = async (secs = 3 * 3600) => {
      backend = installBackend(h.invoke);
      const props = { clips: [clip(1, secs)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
      render(Harness, props);
      await tick();
      return props;
    };

    it('magnifies to 40 pixels per sample, however long the recording', async () => {
      await setup();
      await zoomAllTheWay();
      expect(px()).toBeCloseTo(RATE * 40, 0);
    });

    it('keeps the scrollable width within what a browser can lay out, even for 3 hours at 40 px per sample', async () => {
      await setup();
      await zoomAllTheWay();
      const content = canvas().closest('.content') as HTMLElement;
      // Real width would be 3 h × 44.1 kHz × 40 px ≈ 19 billion px.
      expect(parseFloat(content.style.width)).toBeLessThanOrEqual(8_000_000);
    });

    it('asks the backend for exact samples, not peaks, once individual samples are in reach', async () => {
      await setup();
      await zoomAllTheWay();
      await waitFor(() => expect(backend.of('get_samples').length).toBeGreaterThan(0), { timeout: 3000 });
      const req = backend.of('get_samples').at(-1) as { start: number; end: number };
      expect(req.end - req.start).toBeLessThanOrEqual(1 << 18);
      expect(req.end).toBeGreaterThan(req.start);
    });

    it('a click lands on a whole sample frame', async () => {
      const props = await setup(60);
      await zoomAllTheWay();
      await pointer('pointerdown', 417.3);
      await pointer('pointerup', 417.3);
      const t = props.onseek.mock.calls.at(-1)![0] as number;
      expect(t * RATE).toBeCloseTo(Math.round(t * RATE), 6);
    });

    it('the exact view is drawn from the fetched samples', async () => {
      await setup(60);
      await zoomAllTheWay();
      await waitFor(() => expect(backend.of('get_samples').length).toBeGreaterThan(0), { timeout: 3000 });
      (globalThis as { __canvasCalls?: { fn: string }[] }).__canvasCalls!.length = 0;
      await fireEvent.wheel(canvas().closest('.scroller')!, { deltaY: 10 }); // pan a hair to force a redraw
      await waitFor(() => expect((globalThis as { __canvasCalls?: { fn: string }[] }).__canvasCalls!.some((c) => c.fn === 'arc')).toBe(true));
    });
  });

  it('a drag that starts on a clip edge trims; it never zooms', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    const edge = 100 * FIT(200);
    await pointer('pointerdown', edge - 2);
    await pointer('pointermove', edge - 80);
    await pointer('pointerup', edge - 80);
    expect(props.onchange).toHaveBeenCalled();
  });

  it('dragging right-to-left zooms just the same', async () => {
    backend = installBackend(h.invoke);
    const props = { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() };
    render(Harness, props);
    await tick();
    const fitted = Number(screen.getByTestId('px').textContent);
    await pointer('pointerdown', W * 0.6);
    await pointer('pointermove', W * 0.4);
    await pointer('pointerup', W * 0.4);
    await waitFor(() => expect(Number(screen.getByTestId('px').textContent)).toBeGreaterThan(fitted * 3));
  });
  it('a plain click does not reorder or trim', async () => {
    const { props } = mount([clip(1, 100), clip(2, 100)]);
    await tick();
    await click(50 * FIT(200));
    expect(props.onchange).not.toHaveBeenCalled();
  });

  it('only the primary button interacts', async () => {
    const { props } = mount([clip(1, 100)]);
    await tick();
    await fireEvent(canvas(), new PointerEvent('pointerdown', { clientX: 300, clientY: BODY_Y, button: 2, pointerId: 1, bubbles: true }));
    expect(props.onselect).not.toHaveBeenCalled();
    expect(props.onseek).not.toHaveBeenCalled();
  });
});

describe('timeline drawing and waveform data', () => {
  const calls = () => (globalThis as { __canvasCalls?: { fn: string; args: unknown[] }[] }).__canvasCalls ?? [];

  it('asks the backend for each distinct file once, then for the visible columns', async () => {
    mount([clip(1, 100), clip(2, 100, { path: 'C:/rec/1.aif' })]);
    await waitFor(() => expect(backend.of('get_peaks').length).toBeGreaterThan(0), { timeout: 3000 });
    expect(backend.of('ensure_peaks')).toEqual([{ path: 'C:/rec/1.aif' }]);
    const req = backend.of('get_peaks')[0] as { path: string; start: number; end: number; width: number };
    expect(req.end).toBeGreaterThan(req.start);
    expect(req.width).toBeGreaterThan(0);
    expect(req.width).toBeLessThanOrEqual(4096);
  });

  it('draws the waveform once it arrives', async () => {
    mount([clip(1, 100)]);
    await waitFor(() => expect(calls().filter((c) => c.fn === 'fillRect').length).toBeGreaterThan(20), { timeout: 3000 });
  });

  it('labels the ruler and the clips', async () => {
    mount([clip(1, 100)]);
    await waitFor(() => expect(calls().some((c) => c.fn === 'fillText')).toBe(true));
    const text = calls().filter((c) => c.fn === 'fillText').map((c) => String(c.args[0]));
    expect(text.some((t) => /^0:00$/.test(t))).toBe(true);
    expect(text.some((t) => t.includes('1.AIF'))).toBe(true);
  });

  it('says so on the clip when the waveform cannot be made, and keeps working', async () => {
    h.invoke.mockReset();
    backend = installBackend(h.invoke, { failures: { ensure_peaks: 'cannot read file' } });
    render(Timeline, { clips: [clip(1, 100)], selectedId: null, playheadSecs: 0, playing: false, pxPerSec: 1, onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() });
    await waitFor(() => expect(calls().some((c) => c.fn === 'fillText' && String(c.args[0]).includes('waveform unavailable'))).toBe(true), { timeout: 3000 });
  });

  it('draws a fade envelope over the start and end when fades are set', async () => {
    const fillRects = async (fades: { fadeInSecs?: number; fadeOutSecs?: number }) => {
      (globalThis as { __canvasCalls?: unknown[] }).__canvasCalls!.length = 0;
      backend = installBackend(h.invoke);
      const view = render(Harness, { clips: [clip(1, 100)], onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn(), ...fades });
      await waitFor(() => expect(calls().some((c) => c.fn === 'fillText')).toBe(true));
      await new Promise((r) => setTimeout(r, 150));
      const n = calls().filter((c) => c.fn === 'fillRect').length;
      view.unmount();
      return n;
    };
    const plain = await fillRects({});
    const faded = await fillRects({ fadeInSecs: 20, fadeOutSecs: 20 });
    expect(faded).toBeGreaterThan(plain + 100); // a strip every 2 px across both fades
  });
  it('draws a rubber band and its length while dragging to zoom', async () => {
    mount([clip(1, 100)]);
    await tick();
    (globalThis as { __canvasCalls?: unknown[] }).__canvasCalls!.length = 0;
    await pointer('pointerdown', 200);
    await pointer('pointermove', 600);
    await waitFor(() => expect(calls().some((c) => c.fn === 'fillText' && /^zoom to /.test(String(c.args[0])))).toBe(true));
    await pointer('pointerup', 600);
  });

  it('draws the playhead', async () => {
    mount([clip(1, 100)], { playheadSecs: 30 });
    await waitFor(() => expect(calls().some((c) => c.fn === 'lineTo')).toBe(true));
  });

  it('survives ctrl+wheel zooming', async () => {
    mount([clip(1, 100)]);
    await tick();
    const scroller = canvas().closest('.scroller')!;
    await fireEvent.wheel(scroller, { ctrlKey: true, deltaY: -100, clientX: 400 });
    await fireEvent.wheel(scroller, { ctrlKey: true, deltaY: 100, clientX: 400 });
    expect(canvas()).toBeInTheDocument();
  });

  it('does not fetch waveform data for files whose analysis has not finished', async () => {
    let release!: () => void;
    h.invoke.mockReset();
    backend = installBackend(h.invoke);
    const real = h.invoke.getMockImplementation()!;
    h.invoke.mockImplementation((cmd: string, args: Record<string, unknown>) =>
      cmd === 'ensure_peaks' ? new Promise((r) => (release = () => r({ frames: 1 }))) : real(cmd, args),
    );
    render(Timeline, { clips: [clip(1, 100)], selectedId: null, playheadSecs: 0, playing: false, pxPerSec: 1, onselect: vi.fn(), onseek: vi.fn(), onchange: vi.fn() });
    await new Promise((r) => setTimeout(r, 200));
    expect(backend.calls.filter((c) => c.cmd === 'get_peaks')).toHaveLength(0);
    release();
    await waitFor(() => expect(backend.calls.some((c) => c.cmd === 'get_peaks')).toBe(true), { timeout: 3000 });
  });
});
