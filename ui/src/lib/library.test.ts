import { describe, expect, it } from 'vitest';
import type { FileEntry } from './api';
import { canExport, canRepair, countByStatus, filterEntries, sessionOrder, sortEntries } from './library';

function entry(over: Partial<FileEntry>): FileEntry {
  return {
    path: `C:/r/${over.name ?? 'a.aif'}`,
    name: 'a.aif',
    folder: 'C:/r',
    folder_kind: 'recording',
    size: 1,
    modified_ms: 0,
    status: 'ok',
    detail: null,
    duration_secs: 10,
    sample_rate: 44100,
    bits: 16,
    channels: 2,
    title: null,
    recorded: null,
    ...over,
  };
}

const a = entry({ name: 'a.aif', modified_ms: 300, size: 5, duration_secs: 30 });
const b = entry({ name: 'b.aif', modified_ms: 100, size: 50, duration_secs: 10, status: 'broken', folder_kind: 'temp' });
const c = entry({ name: 'c.aif', modified_ms: 200, size: 20, duration_secs: null, status: 'inuse' });

describe('filterEntries', () => {
  it('filters by status', () => {
    expect(filterEntries([a, b, c], { status: 'broken', query: '' })).toEqual([b]);
    expect(filterEntries([a, b, c], { status: 'all', query: '' })).toHaveLength(3);
  });
  it('searches name case-insensitively', () => {
    expect(filterEntries([a, b, c], { status: 'all', query: 'B.A' })).toEqual([b]);
  });
  it('searches the recorded tag too', () => {
    const r = entry({ name: 'x.aif', recorded: '2025-10-04 10:31:51' });
    expect(filterEntries([a, r], { status: 'all', query: '2025-10' })).toEqual([r]);
  });
});

describe('sortEntries', () => {
  it('sorts by each key in both directions without mutating the input', () => {
    const input = [a, b, c];
    expect(sortEntries(input, 'modified', 'desc').map((e) => e.name)).toEqual(['a.aif', 'c.aif', 'b.aif']);
    expect(sortEntries(input, 'modified', 'asc').map((e) => e.name)).toEqual(['b.aif', 'c.aif', 'a.aif']);
    expect(sortEntries(input, 'size', 'desc').map((e) => e.name)).toEqual(['b.aif', 'c.aif', 'a.aif']);
    expect(sortEntries(input, 'name', 'asc').map((e) => e.name)).toEqual(['a.aif', 'b.aif', 'c.aif']);
    expect(input.map((e) => e.name)).toEqual(['a.aif', 'b.aif', 'c.aif']);
  });
  it('puts missing durations last regardless of direction', () => {
    expect(sortEntries([a, b, c], 'duration', 'asc').map((e) => e.name)).toEqual(['b.aif', 'a.aif', 'c.aif']);
    expect(sortEntries([a, b, c], 'duration', 'desc').map((e) => e.name)).toEqual(['a.aif', 'b.aif', 'c.aif']);
  });
  it('sorts names naturally', () => {
    const files = ['Serato Recording 10.aif', 'Serato Recording 2.aif'].map((name) => entry({ name }));
    expect(sortEntries(files, 'name', 'asc').map((e) => e.name)).toEqual(['Serato Recording 2.aif', 'Serato Recording 10.aif']);
  });
});

describe('sessionOrder', () => {
  it('orders by the recorded tag, falling back to modified time', () => {
    const late = entry({ name: 'late.aif', recorded: '2025-10-04 12:00:00', modified_ms: 1 });
    const early = entry({ name: 'early.aif', recorded: '2025-10-04 09:00:00', modified_ms: 999 });
    const untagged = entry({ name: 'untagged.aif', recorded: null, modified_ms: 5 });
    const order = sessionOrder([late, early, untagged]).map((e) => e.name);
    expect(order.indexOf('early.aif')).toBeLessThan(order.indexOf('late.aif'));
    expect(order).toHaveLength(3);
  });
});

describe('selection rules', () => {
  it('repair needs at least one broken file and nothing in use', () => {
    expect(canRepair([b])).toBe(true);
    expect(canRepair([a, b])).toBe(true);
    expect(canRepair([a])).toBe(false);
    expect(canRepair([b, c])).toBe(false);
    expect(canRepair([])).toBe(false);
  });
  it('export needs at least one readable file', () => {
    expect(canExport([a, b])).toBe(true);
    expect(canExport([a, entry({ status: 'unreadable' })])).toBe(false);
    expect(canExport([entry({ status: 'unsupported' })])).toBe(false);
    expect(canExport([])).toBe(false);
  });
});

describe('countByStatus', () => {
  it('counts each status', () => {
    expect(countByStatus([a, b, c])).toMatchObject({ all: 3, ok: 1, broken: 1, inuse: 1 });
  });
});
