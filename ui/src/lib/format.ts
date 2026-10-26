export function formatDuration(secs: number | null): string {
  if (secs == null) return '—';
  const total = Math.floor(secs);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/** Like formatDuration but with tenths of a second, for trim fields. Truncates, never rounds up. */
export function formatPrecise(secs: number): string {
  const tenths = Math.floor(Math.max(0, secs) * 10 + 1e-6);
  return `${formatDuration(Math.floor(tenths / 10))}.${tenths % 10}`;
}

/** Map a linear peak (0..1) to a 0..100 meter fill on a -54 dB to 0 dB scale. */
export function meterPercent(peak: number): number {
  if (!(peak > 0)) return 0;
  const db = 20 * Math.log10(Math.min(1, peak));
  return Math.max(0, Math.min(100, ((db + 54) / 54) * 100));
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  // whole numbers from MB down, two decimals for GB and up
  return `${i >= 2 ? v.toFixed(2) : Math.round(v)} ${units[i]}`;
}

export function formatRate(hz: number): string {
  return `${parseFloat((hz / 1000).toFixed(2))} kHz`;
}

export function describeFormat(f: { sample_rate: number | null; bits: number | null; channels: number | null }): string {
  if (f.sample_rate == null || f.bits == null || f.channels == null) return '—';
  const ch = f.channels === 1 ? 'mono' : f.channels === 2 ? 'stereo' : `${f.channels} ch`;
  return `${formatRate(f.sample_rate)} · ${f.bits}-bit · ${ch}`;
}

/** Parse "90", "1:30" or "1:02:03.5" into seconds; null if invalid. */
export function parseTime(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const parts = t.split(':');
  if (parts.length > 3 || parts.some((p) => !/^\d+(\.\d+)?$/.test(p))) return null;
  const nums = parts.map(Number);
  if (nums.slice(1).some((n) => n >= 60)) return null;
  return nums.reduce((acc, n) => acc * 60 + n, 0);
}
