/** Timeline model: ordered, trimmed clips of source recordings. All edits return new arrays. */

import type { Clip } from './api';

export interface TClip {
  id: number;
  path: string;
  name: string;
  rate: number;
  /** Length of the whole source recording, in frames. */
  total: number;
  /** Used range of the source, in frames: [start, end). */
  start: number;
  end: number;
  recorded: string | null;
  modified_ms: number;
}

export const frames = (c: TClip) => c.end - c.start;
export const secs = (c: TClip) => frames(c) / c.rate;
export const totalSecs = (cs: TClip[]) => cs.reduce((t, c) => t + secs(c), 0);

export interface Placed {
  clip: TClip;
  /** Start on the timeline, seconds. */
  x: number;
  /** Duration, seconds. */
  w: number;
}

export function place(cs: TClip[]): Placed[] {
  let x = 0;
  return cs.map((clip) => {
    const w = secs(clip);
    const p = { clip, x, w };
    x += w;
    return p;
  });
}

/** Which clip a timeline time (seconds) falls in, and the source frame there. */
export function locate(cs: TClip[], t: number): { index: number; sourceFrame: number } | null {
  if (t < 0) return null;
  let x = 0;
  for (let i = 0; i < cs.length; i++) {
    const w = secs(cs[i]);
    if (t <= x + w && (t < x + w || i === cs.length - 1)) {
      return { index: i, sourceFrame: Math.round(cs[i].start + (t - x) * cs[i].rate) };
    }
    x += w;
  }
  return null;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function trimStart(c: TClip, frame: number): TClip {
  return { ...c, start: clamp(Math.round(frame), 0, c.end - 1) };
}

export function trimEnd(c: TClip, frame: number): TClip {
  return { ...c, end: clamp(Math.round(frame), c.start + 1, c.total) };
}

/** Split clip `index` at a source frame, giving the second half `newId`. No-op if not strictly inside. */
export function splitAt(cs: TClip[], index: number, sourceFrame: number, newId: number): TClip[] {
  const c = cs[index];
  if (!c || sourceFrame <= c.start || sourceFrame >= c.end) return cs;
  return [...cs.slice(0, index), { ...c, end: sourceFrame }, { ...c, id: newId, start: sourceFrame }, ...cs.slice(index + 1)];
}

export function removeAt(cs: TClip[], index: number): TClip[] {
  if (cs.length <= 1 || index < 0 || index >= cs.length) return cs;
  return cs.filter((_, i) => i !== index);
}

export function move(cs: TClip[], from: number, to: number): TClip[] {
  const target = clamp(to, 0, cs.length - 1);
  if (from === target || from < 0 || from >= cs.length) return cs;
  const next = [...cs];
  const [item] = next.splice(from, 1);
  next.splice(target, 0, item);
  return next;
}

export function toBackend(c: TClip): Clip {
  return { path: c.path, start_frame: c.start, end_frame: c.end >= c.total ? null : c.end };
}

const STEPS = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200];
/** Ruler steps below about half a second, as whole numbers of frames so ticks sit exactly on samples. */
const FRAME_STEPS = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000];

/**
 * Smallest round ruler step (seconds) that keeps ticks at least `minPx` apart. Given the sample
 * rate, steps finer than half a second are whole numbers of frames, down to a single sample.
 */
export function niceTickStep(pxPerSec: number, minPx: number, rate?: number): number {
  const steps = rate ? [...FRAME_STEPS.map((n) => n / rate).filter((s) => s < STEPS[0]), ...STEPS] : STEPS;
  return steps.find((s) => s * pxPerSec >= minPx) ?? steps[steps.length - 1];
}

/** Ruler label: a sample number when ticks are closer than 10 ms, else a time with just the digits it needs. */
export function formatTick(t: number, step: number, rate: number): string {
  if (step < 0.01) return `#${Math.round(t * rate)}`;
  const whole = Math.floor(t + 1e-9);
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  const s = String(whole % 60).padStart(2, '0');
  const base = h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
  if (step >= 1) return base;
  const digits = step >= 0.1 ? 1 : 2;
  return `${base}.${String(Math.round((t - whole) * 10 ** digits)).padStart(digits, '0')}`;
}

/**
 * Put a timeline time (seconds) on the sample frame it falls in, using the sample rate of the
 * clip there. The playhead and clicks work in whole frames, so joins can be judged sample by sample.
 */
export function snapToFrame(cs: TClip[], t: number): number {
  const total = totalSecs(cs);
  if (cs.length === 0) return 0;
  const clamped = Math.min(total, Math.max(0, t));
  let x = 0;
  let c = cs[cs.length - 1];
  let at = x;
  for (const clip of cs) {
    const w = secs(clip);
    if (clamped < x + w) {
      c = clip;
      at = x;
      break;
    }
    x += w;
    at = x - w;
  }
  const f = Math.floor((clamped - at) * c.rate + 1e-9);
  return Math.min(total, at + f / c.rate);
}

/** Quarter-sine fade curve, 0 at t = 0 and 1 at t = 1. The same shape the exporter applies. */
export function fadeCurve(t: number): number {
  return Math.sin(Math.min(1, Math.max(0, t)) * (Math.PI / 2));
}
