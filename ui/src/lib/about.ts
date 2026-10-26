import { invoke } from '@tauri-apps/api/core';

/** Where the source lives and where problems are reported. */
export const REPO_URL = 'https://github.com/djryanj/recording-fixer';
export const ISSUES_URL = `${REPO_URL}/issues`;
export const PRIVACY_URL = `${REPO_URL}/blob/main/PRIVACY.md`;

// These call the app and opener plugins' commands directly, through the same `invoke` as the rest of
// the UI, so there is no extra npm package and the test backend can stand in for them.

/** The app's version (from Cargo.toml, so there is one place to bump it). Empty if it cannot be read. */
export async function appVersion(): Promise<string> {
  try {
    return await invoke<string>('plugin:app|version');
  } catch {
    return '';
  }
}

/** Open a web page in the user's browser; failures are ignored (there is nothing useful to do about them). */
export async function openLink(url: string) {
  try {
    await invoke('plugin:opener|open_url', { url });
  } catch {
    /* no browser available */
  }
}
