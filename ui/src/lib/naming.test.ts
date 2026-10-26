import { describe, expect, it } from 'vitest';
import { expand, FILENAME_PRESETS, type NameContext } from './naming';

const ctx: NameContext = {
  name: 'Serato Recording 2',
  recorded: '2025-10-04 21:11:51',
  modifiedMs: Date.parse('2024-01-02T03:04:05'),
  n: 3,
  count: 12,
  ext: 'mp3',
  quality: '320k',
};

describe('expand', () => {
  it('replaces every token', () => {
    expect(expand('{name}', ctx).text).toBe('Serato Recording 2');
    expect(expand('{date}', ctx).text).toBe('2025-10-04');
    expect(expand('{time}', ctx).text).toBe('21-11-51');
    expect(expand('{clock}', ctx).text).toBe('21:11:51');
    expect(expand('{datetime}', ctx).text).toBe('2025-10-04 21-11-51');
    expect(expand('{year}', ctx).text).toBe('2025');
    expect(expand('{n}/{count}', ctx).text).toBe('3/12');
    expect(expand('{n2}', ctx).text).toBe('03');
    expect(expand('{ext} {quality}', ctx).text).toBe('mp3 320k');
  });

  it('falls back to the modified time when there is no recorded tag', () => {
    const c = { ...ctx, recorded: null };
    expect(expand('{date} {time}', c).text).toBe('2024-01-02 03-04-05');
    expect(expand('{year}', c).text).toBe('2024');
  });

  it('keeps literal text and repeats tokens', () => {
    expect(expand('Set {date} - {name} ({date})', ctx).text).toBe('Set 2025-10-04 - Serato Recording 2 (2025-10-04)');
  });

  it('is case-insensitive about token names', () => {
    expect(expand('{NAME}', ctx).text).toBe('Serato Recording 2');
  });

  it('reports unknown tokens and leaves them visible', () => {
    const r = expand('{name} {bogus} {x}', ctx);
    expect(r.unknown).toEqual(['bogus', 'x']);
    expect(r.text).toBe('Serato Recording 2 {bogus} {x}');
  });

  it('handles empty templates and unmatched braces', () => {
    expect(expand('', ctx)).toEqual({ text: '', unknown: [] });
    expect(expand('a { b } c', ctx).text).toBe('a { b } c');
  });

  it('zero pads two digits at most', () => {
    expect(expand('{n2}', { ...ctx, n: 12 }).text).toBe('12');
    expect(expand('{n2}', { ...ctx, n: 123 }).text).toBe('123');
  });
});

describe('FILENAME_PRESETS', () => {
  it('all expand without unknown tokens', () => {
    for (const p of FILENAME_PRESETS) {
      expect(expand(p.template, ctx).unknown, p.label).toEqual([]);
    }
  });
});
