/** Filename and tag templates such as "{date} {time} - {name}". */

export interface NameContext {
  /** Source file name without extension (the first clip's, when combining). */
  name: string;
  /** Serato's "Recorded ..." timestamp, e.g. "2025-10-04 21:11:51". */
  recorded: string | null;
  modifiedMs: number;
  /** 1-based position among the files being exported. */
  n: number;
  count: number;
  ext: string;
  quality: string;
}

export interface Expansion {
  text: string;
  unknown: string[];
}

export const TOKENS: { token: string; help: string }[] = [
  { token: 'name', help: 'source file name' },
  { token: 'date', help: 'recording date, 2025-10-04' },
  { token: 'time', help: 'recording time for file names, 21-11-51' },
  { token: 'clock', help: 'recording time for tags, 21:11:51' },
  { token: 'datetime', help: 'date and time for file names' },
  { token: 'year', help: 'recording year' },
  { token: 'n', help: 'number in the batch' },
  { token: 'n2', help: 'number, zero padded' },
  { token: 'count', help: 'how many files in the batch' },
  { token: 'ext', help: 'format, mp3 / m4a / aif' },
  { token: 'quality', help: 'e.g. 320k or V0' },
];

export const FILENAME_PRESETS: { label: string; template: string }[] = [
  { label: 'Original name', template: '{name}' },
  { label: 'Date and time', template: '{date} {time}' },
  { label: 'Date - name', template: '{date} - {name}' },
  { label: 'Numbered', template: '{name} {n2}' },
  { label: 'Name (quality)', template: '{name} ({quality})' },
];

const pad = (n: number, w = 2) => String(n).padStart(w, '0');

interface Stamp {
  date: string;
  time: string;
  clock: string;
  year: string;
}

function stamp(recorded: string | null, modifiedMs: number): Stamp {
  const m = recorded?.match(/^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2}):(\d{2})/);
  if (m) {
    const [, y, mo, d, h, mi, s] = m;
    return { date: `${y}-${mo}-${d}`, time: `${h}-${mi}-${s}`, clock: `${h}:${mi}:${s}`, year: y };
  }
  const t = new Date(modifiedMs);
  const [y, mo, d, h, mi, s] = [t.getFullYear(), t.getMonth() + 1, t.getDate(), t.getHours(), t.getMinutes(), t.getSeconds()];
  return { date: `${y}-${pad(mo)}-${pad(d)}`, time: `${pad(h)}-${pad(mi)}-${pad(s)}`, clock: `${pad(h)}:${pad(mi)}:${pad(s)}`, year: String(y) };
}

export function expand(template: string, ctx: NameContext): Expansion {
  const st = stamp(ctx.recorded, ctx.modifiedMs);
  const values: Record<string, string> = {
    name: ctx.name,
    date: st.date,
    time: st.time,
    clock: st.clock,
    datetime: `${st.date} ${st.time}`,
    year: st.year,
    n: String(ctx.n),
    n2: pad(ctx.n),
    count: String(ctx.count),
    ext: ctx.ext,
    quality: ctx.quality,
  };
  const unknown: string[] = [];
  const text = template.replace(/\{([A-Za-z0-9_]+)\}/g, (whole, raw: string) => {
    const key = raw.toLowerCase();
    if (key in values) return values[key];
    unknown.push(raw);
    return whole;
  });
  return { text, unknown };
}
