export type Theme = 'system' | 'dark' | 'light';

const KEY = 'theme.v1';
const ORDER: Theme[] = ['system', 'dark', 'light'];

export function nextTheme(t: Theme): Theme {
  return ORDER[(ORDER.indexOf(t) + 1) % ORDER.length];
}

export function isTheme(v: unknown): v is Theme {
  return v === 'system' || v === 'dark' || v === 'light';
}

export function loadTheme(storage: Pick<Storage, 'getItem'> | undefined = globalThis.localStorage): Theme {
  try {
    const v = storage?.getItem(KEY);
    return isTheme(v) ? v : 'system';
  } catch {
    return 'system';
  }
}

/** Apply a theme to the page; `system` removes the override so the OS preference decides. */
export function applyTheme(t: Theme, root: HTMLElement = document.documentElement, storage: Pick<Storage, 'setItem'> | undefined = globalThis.localStorage) {
  if (t === 'system') delete root.dataset.theme;
  else root.dataset.theme = t;
  try {
    storage?.setItem(KEY, t);
  } catch {
    /* the choice just won't be remembered */
  }
}

export const themeLabel: Record<Theme, string> = { system: 'Auto', dark: 'Dark', light: 'Light' };
