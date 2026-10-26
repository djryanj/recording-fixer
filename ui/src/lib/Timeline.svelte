<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { api } from './api';
  import { formatDuration } from './format';
  import { fadeCurve, formatTick, move, niceTickStep, place, snapToFrame, totalSecs, trimEnd, trimStart, type TClip } from './timeline';

  interface Props {
    clips: TClip[];
    selectedId: number | null;
    /** Playback position on the combined timeline, in seconds. */
    playheadSecs: number;
    playing: boolean;
    /** Fade lengths to preview on the timeline, in seconds. */
    fadeInSecs?: number;
    fadeOutSecs?: number;
    /**
     * Show the loudness graph under the waveform: the recordings' short-term loudness, and (with
     * `gainDb`, the planned gain at the start of each second) what it will be after processing.
     * `version` changes when the measurements become available. `null` hides the graph.
     */
    loudness?: { gainDb: number[] | null; target: number | null; version: number } | null;
    pxPerSec: number;
    onselect: (id: number | null) => void;
    onseek: (secs: number) => void;
    onchange: (clips: TClip[]) => void;
  }
  let { clips, selectedId, playheadSecs, playing, fadeInSecs = 0, fadeOutSecs = 0, loudness = null, pxPerSec = $bindable(), onselect, onseek, onchange }: Props = $props();

  const RULER_H = 26;
  const LANE_H = 150;
  /** The loudness graph's lane, and the range of loudness it shows (LUFS). */
  const LOUD_H = 84;
  const LOUD_TOP_LUFS = -5;
  const LOUD_BOTTOM_LUFS = -40;
  const height = $derived(RULER_H + LANE_H + 6 + (loudness ? LOUD_H + 6 : 0));
  const EDGE = 7;
  /** Magnification limit: this many pixels per sample frame, so single samples are easy to pick out. */
  const MAX_PX_PER_FRAME = 40;
  /** Real timeline widths reach billions of pixels when zoomed in on hours of audio, far beyond what
   *  a scrollable element can be; the scrollbar covers at most this many and maps onto the real range. */
  const MAX_SCROLL_PX = 8_000_000;
  /** From this many frames per pixel down, exact samples are fetched and drawn instead of peaks. */
  const FINE_FRAMES_PER_PX = 32;
  /** The backend's cap on frames per get_samples call. */
  const MAX_SAMPLE_FRAMES = 1 << 18;
  /** Height of the strip at the top of each clip that carries its name; dragging it reorders. */
  const TITLE_H = 18;
  /** Pointer travel below which a press is a click rather than a drag. */
  const DRAG_PX = 6;

  interface PeakEntry {
    path: string;
    s: number;
    e: number;
    cols: Int8Array;
  }

  let scroller = $state<HTMLDivElement>();
  let canvas = $state<HTMLCanvasElement>();
  let viewW = $state(800);
  /** Left edge of the view on the real (unscaled) timeline, in pixels. The source of truth for panning. */
  let viewX = $state(0);
  let rev = $state(0); // bumped whenever peak data arrives, to trigger a redraw
  let loading = $state(new Set<string>());
  /** Waveforms still being built: how far each has got, so they can be drawn as they grow. */
  let building = $state(new Map<string, { done: number; total: number }>());
  /** The same for loudness measurements (only drawn while the graph is shown). */
  let measuringLoud = $state(new Map<string, { done: number; total: number }>());
  let failed = $state(new Map<string, string>());
  let dragInfo = $state<{ insertAt: number } | null>(null);
  /** The rubber band while dragging across the waveform to zoom, in view pixels. */
  let marquee = $state<{ x0: number; x1: number } | null>(null);

  /** Exact [min, max] 16-bit values per frame for the frames [s, e) around the view. */
  interface SampleEntry {
    path: string;
    s: number;
    e: number;
    data: Int16Array;
  }

  const entries = new Map<number, PeakEntry>();
  const sampleEntries = new Map<number, SampleEntry>();
  /** Short-term loudness (LUFS) columns per clip, fetched for the part in view. */
  const loudEntries = new Map<number, { path: string; s: number; e: number; cols: Float32Array; stamp: string }>();
  const ready = new Set<string>();
  const inflight = new Set<string>();
  let fetchTimer: ReturnType<typeof setTimeout> | undefined;
  /** True once the real width is known; fitting before that would use the 800 px placeholder. */
  let measured = $state(false);
  /** While true the view keeps everything fitted when the window is resized. */
  let autoFit = true;

  const placed = $derived(place(clips));
  const total = $derived(totalSecs(clips));
  const fitPx = $derived(Math.max(0.0005, (viewW - 2) / Math.max(total, 1)));
  /** The highest sample rate among the clips, which sets how far in we can go. */
  const topRate = $derived(clips.reduce((m, c) => Math.max(m, c.rate), 8000));
  const maxPx = $derived(MAX_PX_PER_FRAME * topRate);
  const realW = $derived(total * pxPerSec);
  const contentW = $derived(Math.max(viewW, Math.min(MAX_SCROLL_PX, Math.ceil(realW))));
  /** Real pixels per scrollbar pixel: 1 until the timeline is too wide to scroll natively. */
  const scrollScale = $derived(realW > contentW && contentW > viewW ? (realW - viewW) / (contentW - viewW) : 1);
  const maxView = $derived(Math.max(0, realW - viewW));

  /** Pan to `x` real pixels from the start, keeping the scrollbar in step. */
  function setView(x: number) {
    viewX = Math.min(maxView, Math.max(0, x));
    if (scroller) scroller.scrollLeft = viewX / scrollScale;
  }

  function onscrollbar() {
    if (!scroller) return;
    // Our own setView lands here too; only a real scrollbar drag differs from where we put it.
    if (Math.abs(scroller.scrollLeft - viewX / scrollScale) > 1) viewX = Math.min(maxView, scroller.scrollLeft * scrollScale);
  }

  export function fit() {
    autoFit = true;
    pxPerSec = fitPx;
    setView(0);
  }

  async function zoomAt(factor: number, anchorPx: number) {
    autoFit = false;
    const next = Math.min(maxPx, Math.max(fitPx, pxPerSec * factor));
    const t = (viewX + anchorPx) / pxPerSec;
    pxPerSec = next;
    await tick();
    setView(t * next - anchorPx);
  }

  export function zoomBy(factor: number) {
    zoomAt(factor, viewW / 2);
  }

  /** Zoom so that the stretch from `t0` to `t1` seconds fills the view. */
  export async function zoomToRange(t0: number, t1: number) {
    autoFit = false;
    const span = Math.max(t1 - t0, 1 / topRate);
    pxPerSec = Math.min(maxPx, Math.max(fitPx, (viewW - 2) / span));
    await tick();
    setView(t0 * pxPerSec);
  }

  // ---- peaks -------------------------------------------------------------

  async function ensure(path: string) {
    if (ready.has(path) || loading.has(path) || failed.has(path)) return;
    loading = new Set(loading).add(path);
    try {
      await api.ensurePeaks(path);
      ready.add(path);
      const left = new Map(building);
      left.delete(path);
      building = left;
    } catch (e) {
      failed = new Map(failed).set(path, String(e));
    } finally {
      const next = new Set(loading);
      next.delete(path);
      loading = next;
    }
    schedule();
  }

  function schedule() {
    clearTimeout(fetchTimer);
    fetchTimer = setTimeout(() => {
      void fetchVisible();
      void fetchLoudness();
    }, 50);
  }

  async function fetchVisible() {
    for (const p of placed) {
      const c = p.clip;
      // A waveform still being built is drawn from what exists so far.
      const progress = ready.has(c.path) ? undefined : building.get(c.path);
      if (!ready.has(c.path) && !(progress && progress.done > 0)) continue;
      const x0 = p.x * pxPerSec - viewX;
      const w = p.w * pxPerSec;
      const visL = Math.max(0, -x0);
      const visR = Math.min(w, viewW - x0);
      if (visR <= visL) continue;
      const margin = (visR - visL) * 0.25;
      const a = c.start + (Math.max(0, visL - margin) / pxPerSec) * c.rate;
      const b = c.start + (Math.min(w, visR + margin) / pxPerSec) * c.rate;
      const s = Math.max(0, Math.floor(a));
      const e = Math.min(progress ? Math.min(c.total, progress.done) : c.total, Math.ceil(b));
      if (e <= s) continue;
      const wantedPerFrame = pxPerSec / c.rate;

      // Close enough to see individual samples: fetch them exactly instead of peaks.
      if (!progress && c.rate / pxPerSec <= FINE_FRAMES_PER_PX && e - s <= MAX_SAMPLE_FRAMES) {
        const got = sampleEntries.get(c.id);
        if (got && got.path === c.path && got.s <= s && got.e >= e) continue;
        const skey = `s|${c.id}|${s}|${e}`;
        if (inflight.has(skey)) continue;
        inflight.add(skey);
        try {
          const data = await api.getSamples(c.path, s, e);
          sampleEntries.set(c.id, { path: c.path, s, e, data });
          rev++;
        } catch (err) {
          failed = new Map(failed).set(c.path, String(err));
        } finally {
          inflight.delete(skey);
        }
        continue;
      }

      const have = entries.get(c.id);
      if (have && have.path === c.path && have.s <= s && have.e >= e && have.cols.length / 2 / (have.e - have.s) >= Math.min(1, wantedPerFrame) * 0.75) continue;
      const cols = Math.min(4096, e - s, Math.max(16, Math.ceil(((e - s) / c.rate) * pxPerSec)));
      const key = `${c.id}|${s}|${e}|${cols}`;
      if (inflight.has(key)) continue;
      inflight.add(key);
      try {
        const data = await api.getPeaks(c.path, s, e, cols);
        entries.set(c.id, { path: c.path, s, e, cols: data });
        rev++;
      } catch (err) {
        failed = new Map(failed).set(c.path, String(err));
      } finally {
        inflight.delete(key);
      }
    }
  }

  async function fetchLoudness() {
    if (!loudness) return;
    const version = loudness.version;
    for (const p of placed) {
      const c = p.clip;
      const prog = measuringLoud.get(c.path);
      const x0 = p.x * pxPerSec - viewX;
      const w = p.w * pxPerSec;
      const visL = Math.max(0, -x0);
      const visR = Math.min(w, viewW - x0);
      if (visR <= visL) continue;
      const margin = (visR - visL) * 0.25;
      const s = Math.max(0, Math.floor(c.start + (Math.max(0, visL - margin) / pxPerSec) * c.rate));
      let e = Math.min(c.total, Math.ceil(c.start + (Math.min(w, visR + margin) / pxPerSec) * c.rate));
      // A measurement still running covers only the start of the recording.
      if (prog && prog.done < prog.total) e = Math.min(e, prog.done);
      if (e <= s) continue;
      const stepFrames = c.rate / 10;
      const cols = Math.min(4096, Math.max(2, Math.ceil((e - s) / stepFrames)), Math.max(16, Math.ceil(((e - s) / c.rate) * pxPerSec)));
      const stamp = `${version}|${prog ? prog.done : ''}`;
      const have = loudEntries.get(c.id);
      if (have && have.path === c.path && have.stamp === stamp && have.s <= s && have.e >= e && have.cols.length / (have.e - have.s) >= (cols / (e - s)) * 0.75) continue;
      const key = `l|${c.id}|${s}|${e}|${cols}|${stamp}`;
      if (inflight.has(key)) continue;
      inflight.add(key);
      try {
        const data = await api.getLoudness(c.path, s, e, cols);
        if (data.length) {
          loudEntries.set(c.id, { path: c.path, s, e: Math.min(e, s + Math.round(data.length * ((e - s) / cols))), cols: data, stamp });
          rev++;
        }
      } catch {
        /* the graph is a nicety: no data, no line */
      } finally {
        inflight.delete(key);
      }
    }
  }

  /** The planned gain (dB) at `t` seconds into the edit list, interpolated between the per-second values. */
  function gainAt(t: number): number {
    const g = loudness?.gainDb;
    if (!g || g.length === 0) return 0;
    const i = Math.max(0, Math.min(g.length - 1, Math.floor(t)));
    const a = g[i];
    const b = g[Math.min(g.length - 1, i + 1)];
    return a + (b - a) * (t - Math.floor(t));
  }

  function drawLoudness(ctx: CanvasRenderingContext2D, col: ReturnType<typeof colours>, W: number) {
    const top = RULER_H + LANE_H + 6;
    const pad = 6;
    const yOf = (lufs: number) => top + pad + ((LOUD_TOP_LUFS - Math.max(LOUD_BOTTOM_LUFS, Math.min(LOUD_TOP_LUFS, lufs))) / (LOUD_TOP_LUFS - LOUD_BOTTOM_LUFS)) * (LOUD_H - 2 * pad);
    ctx.fillStyle = col.inset;
    ctx.fillRect(0, top, W, LOUD_H);
    ctx.font = '500 10px "JetBrains Mono", ui-monospace, monospace';
    ctx.textBaseline = 'middle';
    for (const v of [-10, -20, -30]) {
      ctx.fillStyle = col.line;
      ctx.fillRect(0, Math.round(yOf(v)), W, 1);
      ctx.fillStyle = col.dim;
      ctx.fillText(String(v), 4, yOf(v) - 6);
    }
    const processed = !!loudness?.gainDb;
    placed.forEach((p) => {
      const c = p.clip;
      const x0 = p.x * pxPerSec - viewX;
      const w = p.w * pxPerSec;
      if (x0 > W || x0 + w < 0) return;
      ctx.save();
      ctx.beginPath();
      ctx.rect(Math.max(0, x0), top, Math.min(W, x0 + w) - Math.max(0, x0), LOUD_H);
      ctx.clip();
      const entry = loudEntries.get(c.id);
      if (entry && entry.path === c.path) {
        const n = entry.cols.length;
        const per = (entry.e - entry.s) / n;
        const trace = (shift: boolean) => {
          ctx.beginPath();
          let started = false;
          for (let k = 0; k < n; k++) {
            const frame = entry.s + (k + 0.5) * per;
            if (frame < c.start || frame > c.end) continue;
            const t = p.x + (frame - c.start) / c.rate;
            const v = entry.cols[k];
            const x = x0 + ((frame - c.start) / c.rate) * pxPerSec;
            const y = yOf(v <= -99 ? LOUD_BOTTOM_LUFS : v + (shift ? gainAt(t) : 0));
            if (started) ctx.lineTo(x, y);
            else ctx.moveTo(x, y);
            started = true;
          }
          ctx.stroke();
        };
        ctx.lineJoin = 'round';
        ctx.strokeStyle = col.muted;
        ctx.lineWidth = 1.2;
        trace(false);
        if (processed) {
          ctx.strokeStyle = col.amber;
          ctx.lineWidth = 1.8;
          trace(true);
        }
      }
      // Not measured yet: dim the rest, mark the frontier.
      const prog = measuringLoud.get(c.path);
      if (prog && prog.done < prog.total) {
        const edge = x0 + ((prog.done - c.start) / c.rate) * pxPerSec;
        const from = Math.min(x0 + w, Math.max(x0, edge));
        ctx.fillStyle = col.inset;
        ctx.globalAlpha = 0.55;
        ctx.fillRect(from, top, x0 + w - from, LOUD_H);
        ctx.globalAlpha = 1;
        if (edge >= x0 && edge <= x0 + w) {
          ctx.fillStyle = col.amber;
          ctx.fillRect(edge - 1, top, 2, LOUD_H);
        }
        ctx.fillStyle = col.muted;
        ctx.fillText(`measuring loudness ${Math.floor((prog.done / Math.max(1, prog.total)) * 100)}%`, Math.max(28, x0 + 8), top + LOUD_H - 10);
      }
      ctx.restore();
    });
    // The loudness the processing aims for.
    if (loudness?.target != null) {
      ctx.strokeStyle = col.amber;
      ctx.globalAlpha = 0.5;
      ctx.setLineDash([4, 4]);
      ctx.beginPath();
      ctx.moveTo(0, Math.round(yOf(loudness.target)) + 0.5);
      ctx.lineTo(W, Math.round(yOf(loudness.target)) + 0.5);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
      ctx.fillStyle = col.amber;
      ctx.fillText(`target ${loudness.target} LUFS`, W - 110, yOf(loudness.target) - 7);
    }
    ctx.fillStyle = col.muted;
    ctx.fillText(processed ? 'LOUDNESS, 3 s AVERAGE: grey as recorded, amber after' : 'LOUDNESS, 3 s AVERAGE (LUFS)', 28, top + 8);
    ctx.fillStyle = col.line;
    ctx.fillRect(0, top, W, 1);
  }

  // ---- drawing -----------------------------------------------------------

  function colours() {
    const cs = getComputedStyle(canvas!);
    const v = (n: string) => cs.getPropertyValue(n).trim();
    return {
      text: v('--text'),
      muted: v('--muted'),
      dim: v('--dim'),
      line: v('--line'),
      lineStrong: v('--line-strong'),
      amber: v('--amber'),
      clipA: v('--clip-a'),
      clipB: v('--clip-b'),
      wave: v('--wave'),
      core: v('--wave-core'),
      playhead: v('--playhead'),
      inset: v('--inset'),
    };
  }

  /**
   * Draw exact samples for one clip. Up to a few frames per pixel it is the usual min/max per pixel
   * column; from 2 px per frame each frame is a bar of its own width (a sample held until the
   * next one), joined by a line from about 4 px, with a dot on every sample from 10 px.
   */
  function drawSamples(ctx: CanvasRenderingContext2D, col: ReturnType<typeof colours>, sm: SampleEntry, c: TClip, x0: number, mid: number, amp: number, W: number) {
    const pxPerFrame = pxPerSec / c.rate;
    const y = (v: number) => mid - (v / 32768) * amp;
    const f0 = Math.max(c.start, sm.s);
    const f1 = Math.min(c.end, sm.e);
    const xOf = (f: number) => x0 + (f - c.start) * pxPerFrame;
    if (pxPerFrame < 2) {
      const xFrom = Math.max(0, Math.floor(xOf(f0)));
      const xTo = Math.min(W, Math.ceil(xOf(f1)));
      for (let x = xFrom; x < xTo; x++) {
        const a = Math.max(f0, Math.floor(c.start + (x - x0) / pxPerFrame));
        const b = Math.min(f1, Math.max(a + 1, Math.ceil(c.start + (x + 1 - x0) / pxPerFrame)));
        let lo = 32767;
        let hi = -32768;
        for (let f = a; f < b; f++) {
          const i = 2 * (f - sm.s);
          if (sm.data[i] < lo) lo = sm.data[i];
          if (sm.data[i + 1] > hi) hi = sm.data[i + 1];
        }
        if (lo > hi) continue;
        ctx.fillStyle = col.wave;
        ctx.fillRect(x, y(hi), 1, Math.max(1, y(lo) - y(hi)));
        ctx.fillStyle = col.core;
        ctx.globalAlpha = 0.55;
        ctx.fillRect(x, y(hi * 0.5), 1, Math.max(1, (y(lo) - y(hi)) * 0.5));
        ctx.globalAlpha = 1;
      }
      return;
    }
    const fStart = Math.max(f0, Math.floor(c.start + (0 - x0) / pxPerFrame));
    const fEnd = Math.min(f1, Math.ceil(c.start + (W - x0) / pxPerFrame));
    const bw = Math.max(1, pxPerFrame - (pxPerFrame >= 6 ? 1 : 0));
    ctx.fillStyle = col.wave;
    ctx.globalAlpha = 0.5;
    for (let f = fStart; f < fEnd; f++) {
      const i = 2 * (f - sm.s);
      const top = y(sm.data[i + 1]);
      ctx.fillRect(xOf(f), top, bw, Math.max(1, y(sm.data[i]) - top));
    }
    ctx.globalAlpha = 1;
    if (pxPerFrame >= 4) {
      ctx.strokeStyle = col.core;
      ctx.lineWidth = 1.25;
      for (const off of [1, 0]) {
        ctx.beginPath();
        for (let f = fStart; f < fEnd; f++) {
          const px = xOf(f) + pxPerFrame / 2;
          const py = y(sm.data[2 * (f - sm.s) + off]);
          if (f === fStart) ctx.moveTo(px, py);
          else ctx.lineTo(px, py);
        }
        ctx.stroke();
      }
      ctx.lineWidth = 1;
    }
    if (pxPerFrame >= 10) {
      ctx.fillStyle = col.core;
      for (let f = fStart; f < fEnd; f++) {
        const px = xOf(f) + pxPerFrame / 2;
        for (const off of [1, 0]) {
          ctx.beginPath();
          ctx.arc(px, y(sm.data[2 * (f - sm.s) + off]), 2.5, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    }
  }

  function draw() {
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const W = viewW;
    canvas.width = Math.round(W * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    const col = colours();
    ctx.clearRect(0, 0, W, height);
    ctx.font = '500 11px "JetBrains Mono", ui-monospace, monospace';
    ctx.textBaseline = 'middle';

    // ruler
    ctx.fillStyle = col.inset;
    ctx.fillRect(0, 0, W, RULER_H);
    const rate0 = clips[0]?.rate ?? 44100;
    const step = niceTickStep(pxPerSec, 90, rate0);
    const k0 = Math.floor(viewX / pxPerSec / step);
    ctx.strokeStyle = col.lineStrong;
    ctx.lineWidth = 1;
    for (let k = k0; k * step * pxPerSec - viewX < W; k++) {
      const t = k * step;
      const x = Math.round(t * pxPerSec - viewX) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, RULER_H - 9);
      ctx.lineTo(x, RULER_H);
      ctx.stroke();
      ctx.fillStyle = col.muted;
      ctx.fillText(formatTick(t, step, rate0), x + 4, RULER_H / 2 - 2);
    }
    ctx.strokeStyle = col.line;
    ctx.beginPath();
    ctx.moveTo(0, RULER_H + 0.5);
    ctx.lineTo(W, RULER_H + 0.5);
    ctx.stroke();

    const mid = RULER_H + LANE_H / 2;
    const amp = LANE_H / 2 - 10;
    placed.forEach((p, i) => {
      const c = p.clip;
      const x0 = p.x * pxPerSec - viewX;
      const w = p.w * pxPerSec;
      if (x0 > W || x0 + w < 0) return;
      ctx.fillStyle = i % 2 ? col.clipB : col.clipA;
      ctx.fillRect(x0, RULER_H + 1, w, LANE_H);

      ctx.save();
      ctx.beginPath();
      ctx.rect(Math.max(0, x0), RULER_H + 1, Math.min(W, x0 + w) - Math.max(0, x0), LANE_H);
      ctx.clip();
      // centre line
      ctx.fillStyle = col.line;
      ctx.fillRect(Math.max(0, x0), mid, Math.min(W, x0 + w) - Math.max(0, x0), 1);
      const sm = sampleEntries.get(c.id);
      // Exact samples are used only when they cover everything visible; while a fetch is pending
      // the (blockier) peaks keep the view filled.
      const fa = c.start + (Math.max(0, -x0) / pxPerSec) * c.rate;
      const fb = c.start + (Math.min(w, W - x0) / pxPerSec) * c.rate;
      const exact = !!sm && sm.path === c.path && c.rate / pxPerSec <= FINE_FRAMES_PER_PX && sm.s <= Math.floor(fa) && sm.e >= Math.ceil(fb);
      if (exact) {
        drawSamples(ctx, col, sm!, c, x0, mid, amp, W);
      }
      const pk = exact ? undefined : entries.get(c.id);
      if (pk && pk.path === c.path) {
        const n = pk.cols.length / 2;
        const per = (pk.e - pk.s) / n;
        for (let k = 0; k < n; k++) {
          const f0 = pk.s + k * per;
          const f1 = f0 + per;
          if (f1 < c.start || f0 > c.end) continue;
          const xa = x0 + ((Math.max(f0, c.start) - c.start) / c.rate) * pxPerSec; // frames → px
          const xb = x0 + ((Math.min(f1, c.end) - c.start) / c.rate) * pxPerSec;
          if (xb < 0 || xa > W) continue;
          const bw = Math.max(1, xb - xa + 0.5);
          const hi = (pk.cols[2 * k + 1] / 128) * amp;
          const lo = (pk.cols[2 * k] / 128) * amp;
          ctx.fillStyle = col.wave;
          ctx.fillRect(xa, mid - hi, bw, Math.max(1, hi - lo));
          // a brighter core, like the glow of a phosphor display
          ctx.fillStyle = col.core;
          ctx.globalAlpha = 0.55;
          ctx.fillRect(xa, mid - hi * 0.5, bw, Math.max(1, (hi - lo) * 0.5));
          ctx.globalAlpha = 1;
        }
      }
      // A waveform that is still being built: dim what is not there yet and mark how far it has got.
      const grow = building.get(c.path);
      if (grow && !ready.has(c.path)) {
        const edge = x0 + ((grow.done - c.start) / c.rate) * pxPerSec;
        const from = Math.min(x0 + w, Math.max(x0, edge));
        ctx.fillStyle = col.inset;
        ctx.globalAlpha = 0.55;
        ctx.fillRect(from, RULER_H + 1, x0 + w - from, LANE_H);
        ctx.globalAlpha = 1;
        if (edge >= x0 && edge <= x0 + w) {
          ctx.fillStyle = col.amber;
          ctx.fillRect(edge - 1, RULER_H + 1, 2, LANE_H);
        }
      }
      // title strip: the name, and a grip to show that it can be dragged to reorder
      const pct = grow ? Math.min(99, Math.floor((grow.done / Math.max(1, grow.total)) * 100)) : null;
      const status = failed.get(c.path)
        ? '  [waveform unavailable]'
        : loading.has(c.path)
          ? pct != null && !ready.has(c.path)
            ? `  [building waveform ${pct}%]`
            : '  [analysing…]'
          : '';
      ctx.font = '600 11px "Barlow Condensed", "Barlow", sans-serif';
      const label = `${String(i + 1).padStart(2, '0')}  ${c.name.toUpperCase()}${status}`;
      ctx.fillStyle = col.inset;
      ctx.globalAlpha = 0.82;
      ctx.fillRect(Math.max(0, x0), RULER_H + 1, Math.min(W, x0 + w) - Math.max(0, x0), TITLE_H);
      ctx.globalAlpha = 1;
      ctx.fillStyle = c.id === selectedId ? col.amber : col.dim;
      for (let gx = 0; gx < 2; gx++) for (let gy = 0; gy < 3; gy++) ctx.fillRect(x0 + 6 + gx * 4, RULER_H + 5 + gy * 4, 2, 2);
      ctx.fillStyle = col.text;
      ctx.fillText(label, Math.max(22, x0 + 22), RULER_H + 10);
      ctx.font = '500 11px "JetBrains Mono", ui-monospace, monospace';
      ctx.restore();

      // boundary between clips, and selection outline with trim handles
      ctx.strokeStyle = col.lineStrong;
      ctx.beginPath();
      ctx.moveTo(Math.round(x0) + 0.5, RULER_H + 1);
      ctx.lineTo(Math.round(x0) + 0.5, RULER_H + LANE_H);
      ctx.stroke();
      if (c.id === selectedId) {
        ctx.strokeStyle = col.amber;
        ctx.lineWidth = 2;
        ctx.strokeRect(x0 + 1, RULER_H + 2, w - 2, LANE_H - 2);
        ctx.lineWidth = 1;
        ctx.fillStyle = col.amber;
        ctx.fillRect(x0, mid - 16, 5, 32);
        ctx.fillRect(x0 + w - 5, mid - 16, 5, 32);
      }
    });

    // fade envelopes: dim the lane by how much quieter the exported audio will be there
    const dim = (fromSec: number, lenSec: number, rising: boolean) => {
      if (!(lenSec > 0) || total <= 0) return;
      const len = Math.min(lenSec, total);
      ctx.fillStyle = col.inset;
      for (let x = Math.max(0, fromSec * pxPerSec - viewX); x < Math.min(W, (fromSec + len) * pxPerSec - viewX); x += 2) {
        const t = (x + viewX) / pxPerSec - fromSec;
        const g = fadeCurve(rising ? t / len : 1 - t / len);
        ctx.globalAlpha = (1 - g) * 0.8;
        ctx.fillRect(x, RULER_H + 1, 2, LANE_H);
      }
      ctx.globalAlpha = 1;
    };
    dim(0, fadeInSecs, true);
    dim(Math.max(0, total - fadeOutSecs), fadeOutSecs, false);

    // playhead
    const px = playheadSecs * pxPerSec - viewX;
    if (px >= -1 && px <= W + 1) {
      ctx.strokeStyle = col.playhead;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(Math.round(px) + 0.5, 0);
      ctx.lineTo(Math.round(px) + 0.5, RULER_H + LANE_H);
      ctx.stroke();
      ctx.lineWidth = 1;
      ctx.fillStyle = col.playhead;
      ctx.beginPath();
      ctx.moveTo(px - 6, 0);
      ctx.lineTo(px + 6, 0);
      ctx.lineTo(px, 9);
      ctx.closePath();
      ctx.fill();
    }

    if (dragInfo) {
      const target = placed[dragInfo.insertAt];
      const x = (target ? target.x : total) * pxPerSec - viewX;
      ctx.fillStyle = col.amber;
      ctx.fillRect(x - 1.5, RULER_H, 3, LANE_H);
    }

    // the zoom rubber band, with the length of the stretch it will zoom to
    if (marquee) {
      const span = (marquee.x1 - marquee.x0) / pxPerSec;
      ctx.fillStyle = col.amber;
      ctx.globalAlpha = 0.16;
      ctx.fillRect(marquee.x0, RULER_H, marquee.x1 - marquee.x0, LANE_H);
      ctx.globalAlpha = 1;
      ctx.strokeStyle = col.amber;
      ctx.strokeRect(marquee.x0 + 0.5, RULER_H + 0.5, marquee.x1 - marquee.x0, LANE_H - 1);
      ctx.fillStyle = col.amber;
      ctx.font = '600 11px "JetBrains Mono", ui-monospace, monospace';
      const r = clips[0]?.rate ?? 44100;
      ctx.fillText(span < 1 ? `zoom to ${Math.max(1, Math.round(span * r))} samples` : `zoom to ${formatDuration(span)}`, marquee.x0 + 6, RULER_H + LANE_H - 10);
    }
    if (loudness) drawLoudness(ctx, col, W);
  }

  $effect(() => {
    // Everything the drawing depends on, read here so the effect re-runs.
    void [placed, selectedId, playheadSecs, pxPerSec, viewX, viewW, rev, loading, building, measuringLoud, failed, dragInfo, marquee, fadeInSecs, fadeOutSecs, loudness];
    draw();
  });

  $effect(() => {
    void [placed, pxPerSec, viewX, viewW];
    for (const p of new Set(clips.map((c) => c.path))) ensure(p);
    schedule();
  });

  $effect(() => {
    void [loudness?.version, !!loudness];
    schedule();
  });

  // Fit on the first measurement and on window resizes, but not when the clips change: that would
  // move the timeline under the pointer in the middle of a trim drag.
  $effect(() => {
    void viewW;
    if (measured && autoFit) {
      untrack(() => {
        if (total > 0) pxPerSec = Math.max(0.0005, (viewW - 2) / total);
      });
    }
  });

  // While playing, keep the playhead in view.
  $effect(() => {
    if (!playing || !scroller) return;
    const x = playheadSecs * pxPerSec;
    if (x < viewX || x > viewX + viewW - 40) setView(x - viewW * 0.2);
  });

  // ---- pointer interaction -----------------------------------------------

  // What a press on the timeline can turn into:
  //   edge       drag a clip's edge to trim it
  //   title      drag a clip's name strip to reorder it
  //   zoom       drag across a waveform to zoom into that stretch
  //   (a press that never moves is a click: the playhead goes there and plays)
  type Drag =
    | { kind: 'trim-start' | 'trim-end'; index: number; startX: number; orig: TClip }
    | { kind: 'title'; index: number; startX: number; moved: boolean }
    | { kind: 'zoom'; index: number; startX: number; moved: boolean };
  let drag: Drag | null = null;

  const localX = (e: PointerEvent) => e.clientX - canvas!.getBoundingClientRect().left;
  const localY = (e: PointerEvent) => e.clientY - canvas!.getBoundingClientRect().top;
  const timeAt = (x: number) => Math.max(0, Math.min(total, (x + viewX) / pxPerSec));
  /** Where a click at `x` puts the playhead: on the exact sample frame under the pointer. */
  const frameAt = (x: number) => snapToFrame(clips, timeAt(x));

  type Part = 'start' | 'end' | 'title' | 'body';

  function hit(x: number, y: number): { index: number; part: Part } | null {
    for (let i = 0; i < placed.length; i++) {
      const p = placed[i];
      const x0 = p.x * pxPerSec - viewX;
      const x1 = x0 + p.w * pxPerSec;
      if (x < x0 - EDGE || x > x1 + EDGE) continue;
      // At a shared boundary, left of it belongs to the earlier clip's end, right of it to the next clip's start.
      if (Math.abs(x - x1) <= EDGE && x <= x1) return { index: i, part: 'end' };
      if (Math.abs(x - x0) <= EDGE && (x >= x0 || i === 0)) return { index: i, part: 'start' };
      if (x >= x0 && x <= x1) return { index: i, part: y < RULER_H + TITLE_H ? 'title' : 'body' };
    }
    return null;
  }

  // Two-finger pinch on a touchscreen. Fingers are tracked by pointer id; while two are down the
  // gesture only zooms (it never trims, reorders or seeks), and it stays inert until all lift.
  const touches = new Map<number, number>();
  let pinchLast = 0;
  let pinching = false;
  const pinchSpan = () => {
    const xs = [...touches.values()];
    return { dist: Math.abs(xs[0] - xs[1]), mid: (xs[0] + xs[1]) / 2 };
  };

  function pinchDown(e: PointerEvent): boolean {
    if (e.pointerType !== 'touch') return false;
    touches.set(e.pointerId, localX(e));
    if (touches.size < 2) return pinching;
    pinching = true;
    drag = null;
    dragInfo = null;
    marquee = null;
    pinchLast = pinchSpan().dist;
    return true;
  }

  function pinchMove(e: PointerEvent): boolean {
    if (e.pointerType !== 'touch' || !touches.has(e.pointerId)) return false;
    touches.set(e.pointerId, localX(e));
    if (touches.size < 2) return pinching;
    const { dist, mid } = pinchSpan();
    if (pinchLast > 8 && dist > 8) void zoomAt(dist / pinchLast, mid);
    pinchLast = dist;
    return true;
  }

  function pinchUp(e: PointerEvent): boolean {
    if (e.pointerType !== 'touch') return false;
    touches.delete(e.pointerId);
    const was = pinching;
    if (touches.size === 0) pinching = false;
    return was;
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    if (pinchDown(e)) return;
    canvas!.focus();
    const x = localX(e);
    const y = localY(e);
    if (y < RULER_H) {
      onseek(frameAt(x)); // clicking the ruler plays from there
      return;
    }
    const h = hit(x, y);
    if (!h) {
      onselect(null);
      return;
    }
    const c = clips[h.index];
    onselect(c.id);
    try {
      canvas!.setPointerCapture(e.pointerId);
    } catch {
      /* capture is only a convenience; dragging still works inside the canvas */
    }
    if (h.part === 'start' || h.part === 'end') {
      drag = { kind: h.part === 'start' ? 'trim-start' : 'trim-end', index: h.index, startX: x, orig: c };
    } else {
      drag = { kind: h.part === 'title' ? 'title' : 'zoom', index: h.index, startX: x, moved: false };
    }
  }

  function onpointermove(e: PointerEvent) {
    if (pinchMove(e)) return;
    const x = localX(e);
    const y = localY(e);
    if (!drag) {
      const h = y < RULER_H ? null : hit(x, y);
      canvas!.style.cursor = y < RULER_H ? 'pointer' : !h ? 'default' : h.part === 'body' ? 'crosshair' : h.part === 'title' ? 'grab' : 'ew-resize';
      return;
    }
    if (drag.kind === 'title') {
      if (!drag.moved && Math.abs(x - drag.startX) < DRAG_PX) return;
      drag.moved = true;
      canvas!.style.cursor = 'grabbing';
      const t = timeAt(x);
      let at = placed.findIndex((p) => t < p.x + p.w / 2);
      if (at < 0) at = placed.length;
      dragInfo = { insertAt: at };
      return;
    }
    if (drag.kind === 'zoom') {
      if (!drag.moved && Math.abs(x - drag.startX) < DRAG_PX) return;
      drag.moved = true;
      canvas!.style.cursor = 'zoom-in';
      marquee = { x0: Math.max(0, Math.min(x, drag.startX)), x1: Math.min(viewW, Math.max(x, drag.startX)) };
      return;
    }
    const dFrames = ((x - drag.startX) / pxPerSec) * drag.orig.rate;
    const next = drag.kind === 'trim-start' ? trimStart(drag.orig, drag.orig.start + dFrames) : trimEnd(drag.orig, drag.orig.end + dFrames);
    onchange(clips.map((c, i) => (i === drag!.index ? next : c)));
  }

  function onpointerup(e: PointerEvent) {
    if (pinchUp(e)) return;
    const d = drag;
    if (d?.kind === 'title') {
      if (d.moved && dragInfo) {
        let to = dragInfo.insertAt;
        if (to > d.index) to -= 1; // removing the dragged clip shifts later positions down
        onchange(move(clips, d.index, to));
      } else {
        onseek(frameAt(localX(e)));
      }
    } else if (d?.kind === 'zoom') {
      if (d.moved && marquee) {
        void zoomToRange((marquee.x0 + viewX) / pxPerSec, (marquee.x1 + viewX) / pxPerSec);
      } else {
        onseek(frameAt(localX(e))); // a plain click plays from there
      }
    }
    drag = null;
    dragInfo = null;
    marquee = null;
    try {
      if (canvas?.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
    } catch {
      /* nothing captured */
    }
  }

  onMount(() => {
    const ro = new ResizeObserver(() => {
      if (scroller) viewW = scroller.clientWidth;
    });
    ro.observe(scroller!);
    viewW = scroller!.clientWidth;
    measured = true;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        // A trackpad pinch arrives as a stream of ctrl+wheel events with small deltas; a mouse wheel
        // notch is ~100. Scaling by delta gives smooth pinching and about 1.25x per notch.
        e.preventDefault();
        const px = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
        const factor = Math.min(2, Math.max(0.5, Math.exp(-px * 0.0022)));
        zoomAt(factor, e.clientX - scroller!.getBoundingClientRect().left);
      } else {
        // Pan in real pixels so a wheel notch moves a few samples when zoomed right in.
        e.preventDefault();
        setView(viewX + (Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY));
      }
    };
    scroller!.addEventListener('wheel', onWheel, { passive: false });
    // The backend reports how far each waveform build has got; draw what exists.
    let offPeaks = () => {};
    let unmounted = false;
    void api
      .onPeaksProgress((p) => {
        if (ready.has(p.path)) return;
        building = new Map(building).set(p.path, { done: p.done, total: p.total });
        schedule();
      })
      .then((un) => (unmounted ? un() : (offPeaks = un)));
    let offLoud = () => {};
    void api
      .onLoudnessProgress((p) => {
        measuringLoud = new Map(measuringLoud).set(p.path, { done: p.done, total: p.total });
        schedule();
      })
      .then((un) => (unmounted ? un() : (offLoud = un)));
    return () => {
      unmounted = true;
      offPeaks();
      offLoud();
      ro.disconnect();
      scroller?.removeEventListener('wheel', onWheel);
      clearTimeout(fetchTimer);
    };
  });
</script>

<div class="scroller" bind:this={scroller} onscroll={onscrollbar}>
  <div class="content" style="width:{contentW}px; height:{height}px">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <canvas
      bind:this={canvas}
      data-testid="timeline"
      style="width:{viewW}px; height:{height}px"
      tabindex="0"
      aria-label="Waveform timeline. Click to play from that point, drag across a waveform to zoom in, drag a clip's name bar to reorder it, drag its edges to trim it. Use the buttons to split and remove."
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      onpointercancel={onpointerup}
    ></canvas>
  </div>
</div>

<style>
  .scroller {
    min-width: 0;
    max-width: 100%;
    overflow-x: auto;
    overflow-y: hidden;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: var(--inset);
  }
  .content {
    position: relative;
  }
  canvas {
    position: sticky;
    left: 0;
    display: block;
    outline: none;
    touch-action: none;
  }
  canvas:focus-visible {
    outline: 1px solid var(--amber);
    outline-offset: -1px;
  }
</style>
