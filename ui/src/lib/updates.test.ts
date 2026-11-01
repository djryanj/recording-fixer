// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { CHECK_INTERVAL_MS, checkForUpdate, checkIsDue, setUpdateChecks, UPDATE_KEY, updateChecksEnabled } from './updates';

beforeEach(() => {
  localStorage.clear();
  h.invoke.mockReset();
});

describe('update check preference', () => {
  it('is on by default and can be switched off and on', () => {
    expect(updateChecksEnabled()).toBe(true);
    setUpdateChecks(false);
    expect(updateChecksEnabled()).toBe(false);
    setUpdateChecks(true);
    expect(updateChecksEnabled()).toBe(true);
  });

  it('is due on first run, then not for a day, then due again', async () => {
    expect(checkIsDue()).toBe(true);
    h.invoke.mockResolvedValue({ current: '0.1.0', latest: '0.1.0', url: null, newer: false });
    await checkForUpdate();
    expect(checkIsDue()).toBe(false);
    expect(checkIsDue(Date.now() + CHECK_INTERVAL_MS + 1000)).toBe(true);
  });

  it('is never due automatically when switched off', () => {
    setUpdateChecks(false);
    expect(checkIsDue()).toBe(false);
  });

  it('stores only the switch and a time, never the answer', async () => {
    h.invoke.mockResolvedValue({ current: '0.1.0', latest: '9.9.9', url: 'https://github.com/x', newer: true });
    await checkForUpdate();
    expect(Object.keys(JSON.parse(localStorage.getItem(UPDATE_KEY)!))).toEqual(['lastChecked']);
  });

  it('does not count a failed check as done, so it is tried again', async () => {
    h.invoke.mockRejectedValue('offline');
    await expect(checkForUpdate()).rejects.toBe('offline');
    expect(checkIsDue()).toBe(true);
  });
});
