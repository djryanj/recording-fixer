import { describe, expect, it } from 'vitest';
import {
  fadeCurve,
  formatTick,
  locate,
  move,
  niceTickStep,
  place,
  removeAt,
  snapToFrame,
  splitAt,
  toBackend,
  totalSecs,
  trimEnd,
  trimStart,
  type TClip,
} from './timeline';

const RATE = 44100;
const clip = (id: number, total = RATE * 100, start = 0, end = total): TClip => ({
  id,
  path: `C:/r/${id}.aif`,
  name: `${id}.aif`,
  rate: RATE,
  total,
  start,
  end,
  recorded: null,
  modified_ms: 0,
});

describe('layout', () => {
  it('places clips back to back in seconds', () => {
    const placed = place([clip(1, RATE * 10), clip(2, RATE * 30, RATE * 5, RATE * 15)]);
    expect(placed.map((p) => [p.x, p.w])).toEqual([
      [0, 10],
      [10, 10],
    ]);
    expect(totalSecs([clip(1, RATE * 10), clip(2, RATE * 30, RATE * 5, RATE * 15)])).toBe(20);
  });

  it('locates a timeline position inside a clip', () => {
    const cs = [clip(1, RATE * 10), clip(2, RATE * 30, RATE * 5, RATE * 15)];
    expect(locate(cs, 3)).toEqual({ index: 0, sourceFrame: RATE * 3 });
    expect(locate(cs, 12)).toEqual({ index: 1, sourceFrame: RATE * 7 });
    expect(locate(cs, 20)).toEqual({ index: 1, sourceFrame: RATE * 15 });
    expect(locate(cs, -1)).toBeNull();
    expect(locate(cs, 25)).toBeNull();
  });
});

describe('trim', () => {
  it('moves the start and end inside the source and never crosses', () => {
    const c = clip(1, 1000, 100, 900);
    expect(trimStart(c, 200).start).toBe(200);
    expect(trimStart(c, -50).start).toBe(0);
    expect(trimStart(c, 5000).start).toBe(899);
    expect(trimEnd(c, 800).end).toBe(800);
    expect(trimEnd(c, 5000).end).toBe(1000);
    expect(trimEnd(c, 0).end).toBe(101);
  });

  it('does not mutate the original', () => {
    const c = clip(1, 1000);
    trimStart(c, 10);
    expect(c.start).toBe(0);
  });
});

describe('split / remove / move', () => {
  it('splits one clip into two that cover the same audio', () => {
    const cs = [clip(1, 1000, 100, 900)];
    const out = splitAt(cs, 0, 500, 7);
    expect(out).toHaveLength(2);
    expect([out[0].start, out[0].end, out[1].start, out[1].end]).toEqual([100, 500, 500, 900]);
    expect(out[0].id).toBe(1);
    expect(out[1].id).toBe(7);
    expect(out[1].path).toBe(out[0].path);
  });

  it('refuses to split at or beyond the edges', () => {
    const cs = [clip(1, 1000, 100, 900)];
    expect(splitAt(cs, 0, 100, 2)).toBe(cs);
    expect(splitAt(cs, 0, 900, 2)).toBe(cs);
    expect(splitAt(cs, 0, 50, 2)).toBe(cs);
    expect(splitAt(cs, 5, 500, 2)).toBe(cs);
  });

  it('removes a clip but never the last one', () => {
    const cs = [clip(1), clip(2)];
    expect(removeAt(cs, 0).map((c) => c.id)).toEqual([2]);
    const one = [clip(1)];
    expect(removeAt(one, 0)).toBe(one);
    expect(removeAt(cs, 9)).toBe(cs);
  });

  it('moves clips and clamps the target', () => {
    const cs = [clip(1), clip(2), clip(3)];
    expect(move(cs, 0, 2).map((c) => c.id)).toEqual([2, 3, 1]);
    expect(move(cs, 2, 0).map((c) => c.id)).toEqual([3, 1, 2]);
    expect(move(cs, 1, 99).map((c) => c.id)).toEqual([1, 3, 2]);
    expect(move(cs, 1, 1)).toBe(cs);
  });
});

describe('toBackend', () => {
  it('uses null for "to the end" so the file length is authoritative', () => {
    expect(toBackend(clip(1, 1000, 0, 1000))).toEqual({ path: 'C:/r/1.aif', start_frame: 0, end_frame: null });
    expect(toBackend(clip(1, 1000, 10, 600))).toEqual({ path: 'C:/r/1.aif', start_frame: 10, end_frame: 600 });
  });
});

describe('niceTickStep', () => {
  it('picks a round step with at least the minimum pixel spacing', () => {
    expect(niceTickStep(100, 80)).toBe(1);
    expect(niceTickStep(10, 80)).toBe(10);
    expect(niceTickStep(0.1, 80)).toBeGreaterThanOrEqual(5);
    for (const pxPerSec of [0.05, 0.3, 1, 8, 40, 200]) {
      const step = niceTickStep(pxPerSec, 80);
      expect(step * pxPerSec).toBeGreaterThanOrEqual(80);
    }
  });
});

describe('ticks down to single samples', () => {
  it('with a sample rate, steps are whole numbers of frames until they reach a second or so', () => {
    const rate = 44100;
    // 40 px per frame: the smallest step is one frame.
    expect(niceTickStep(rate * 40, 30, rate)).toBeCloseTo(1 / rate, 12);
    // 0.3 px per frame, 60 px minimum: 200 frames.
    const step = niceTickStep(rate * 0.3, 60, rate);
    expect(Math.round(step * rate)).toBe(200);
    expect(step * rate * 0.3).toBeGreaterThanOrEqual(60);
    // Zoomed far out it is still the old whole-second ladder.
    expect(niceTickStep(0.05, 80, rate)).toBe(niceTickStep(0.05, 80));
  });

  it('labels ticks as sample numbers when they are closer than 10 ms', () => {
    expect(formatTick(1.5, 1 / 44100, 44100)).toBe('#66150');
    expect(formatTick(65.25, 0.05, 44100)).toBe('1:05.25');
    expect(formatTick(65, 5, 44100)).toBe('1:05');
  });
});

describe('snapToFrame', () => {
  const a = { ...clip(1, 10_000), rate: 1000 };
  it('puts a time on the frame under it, never on the next one', () => {
    expect(snapToFrame([a], 0.0019)).toBeCloseTo(0.001, 9);
    expect(snapToFrame([a], 0.002)).toBeCloseTo(0.002, 9);
    expect(snapToFrame([a], 0.0029999999)).toBeCloseTo(0.002, 9);
  });
  it('uses the rate of the clip the time falls in', () => {
    const b = { ...clip(2, 1000), rate: 100 };
    expect(snapToFrame([a, b], 10.019)).toBeCloseTo(10.01, 9);
  });
  it('clamps to the timeline', () => {
    expect(snapToFrame([a], -1)).toBe(0);
    expect(snapToFrame([a], 99)).toBeCloseTo(10, 9);
  });
});

describe('fadeCurve', () => {
  it('runs from silence to full level, equal power in the middle, and clamps outside', () => {
    expect(fadeCurve(0)).toBe(0);
    expect(fadeCurve(1)).toBe(1);
    expect(fadeCurve(0.5)).toBeCloseTo(Math.SQRT1_2, 6);
    expect(fadeCurve(-3)).toBe(0);
    expect(fadeCurve(7)).toBe(1);
    const xs = Array.from({ length: 101 }, (_, i) => fadeCurve(i / 100));
    expect(xs.every((v, i) => i === 0 || v >= xs[i - 1])).toBe(true);
  });
});