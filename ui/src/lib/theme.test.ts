import { describe, expect, it } from 'vitest';
import { applyTheme, isTheme, loadTheme, nextTheme } from './theme';

class Mem {
  data = new Map<string, string>();
  getItem = (k: string) => this.data.get(k) ?? null;
  setItem = (k: string, v: string) => void this.data.set(k, v);
}

describe('theme', () => {
  it('cycles system -> dark -> light -> system', () => {
    expect(nextTheme('system')).toBe('dark');
    expect(nextTheme('dark')).toBe('light');
    expect(nextTheme('light')).toBe('system');
  });

  it('recognises only the three themes', () => {
    expect(['system', 'dark', 'light'].every(isTheme)).toBe(true);
    expect([null, '', 'blue', 3, undefined].some(isTheme)).toBe(false);
  });

  it('loads a saved theme and falls back to system for junk or failure', () => {
    const s = new Mem();
    expect(loadTheme(s)).toBe('system');
    s.setItem('theme.v1', 'light');
    expect(loadTheme(s)).toBe('light');
    s.setItem('theme.v1', 'neon');
    expect(loadTheme(s)).toBe('system');
    expect(loadTheme({ getItem: () => { throw new Error('blocked'); } })).toBe('system');
  });

  it('applies and persists, and `system` clears the override', () => {
    const root = { dataset: {} as Record<string, string> } as unknown as HTMLElement;
    const s = new Mem();
    applyTheme('light', root, s);
    expect(root.dataset.theme).toBe('light');
    expect(s.getItem('theme.v1')).toBe('light');
    applyTheme('system', root, s);
    expect(root.dataset.theme).toBeUndefined();
    expect(s.getItem('theme.v1')).toBe('system');
  });

  it('survives storage that throws', () => {
    const root = { dataset: {} as Record<string, string> } as unknown as HTMLElement;
    expect(() => applyTheme('dark', root, { setItem: () => { throw new Error('full'); } })).not.toThrow();
    expect(root.dataset.theme).toBe('dark');
  });
});
