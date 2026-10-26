import type { FileEntry, Status } from './api';

export type StatusFilter = 'all' | Status;
export type SortKey = 'name' | 'status' | 'duration' | 'recorded' | 'modified' | 'size';
export type SortDir = 'asc' | 'desc';

export function filterEntries(entries: FileEntry[], f: { status: StatusFilter; query: string }): FileEntry[] {
  const q = f.query.trim().toLowerCase();
  return entries.filter(
    (e) =>
      (f.status === 'all' || e.status === f.status) &&
      (!q || e.name.toLowerCase().includes(q) || (e.recorded ?? '').toLowerCase().includes(q)),
  );
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

export function sortEntries(entries: FileEntry[], key: SortKey, dir: SortDir): FileEntry[] {
  const sign = dir === 'asc' ? 1 : -1;
  const num = (v: number | null) => v;
  return [...entries].sort((x, y) => {
    let r = 0;
    switch (key) {
      case 'name':
        r = collator.compare(x.name, y.name);
        break;
      case 'status':
        r = x.status.localeCompare(y.status);
        break;
      case 'duration': {
        // missing values always sort last
        const [a, b] = [num(x.duration_secs), num(y.duration_secs)];
        if (a == null || b == null) return a == null ? (b == null ? 0 : 1) : -1;
        r = a - b;
        break;
      }
      case 'recorded':
        r = (x.recorded ?? '').localeCompare(y.recorded ?? '') || x.modified_ms - y.modified_ms;
        break;
      case 'modified':
        r = x.modified_ms - y.modified_ms;
        break;
      case 'size':
        r = x.size - y.size;
        break;
    }
    return r * sign;
  });
}

/** The order a DJ most likely wants for gluing a night together: earliest first. */
export function sessionOrder(entries: FileEntry[]): FileEntry[] {
  return [...entries].sort((x, y) => {
    const a = x.recorded ?? '';
    const b = y.recorded ?? '';
    if (a && b) return a.localeCompare(b);
    return x.modified_ms - y.modified_ms;
  });
}

export function canRepair(sel: FileEntry[]): boolean {
  return sel.some((e) => e.status === 'broken') && !sel.some((e) => e.status === 'inuse');
}

export function canExport(sel: FileEntry[]): boolean {
  return sel.length > 0 && sel.every((e) => e.status !== 'unreadable' && e.status !== 'unsupported');
}

export function countByStatus(entries: FileEntry[]): Record<StatusFilter, number> {
  const c: Record<StatusFilter, number> = { all: entries.length, ok: 0, broken: 0, inuse: 0, unreadable: 0, unsupported: 0 };
  for (const e of entries) c[e.status]++;
  return c;
}
