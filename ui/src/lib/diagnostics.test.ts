// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn(async (..._a: unknown[]) => null) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { installErrorLogging } from './diagnostics';

describe('error logging', () => {
  it('records uncaught errors and unhandled rejections in the app log', () => {
    const win = new EventTarget() as unknown as Window;
    installErrorLogging(win);
    const err = Object.assign(new Event('error'), { message: 'boom', filename: 'app.js', lineno: 7 });
    win.dispatchEvent(err);
    const rej = Object.assign(new Event('unhandledrejection'), { reason: new Error('nope') });
    win.dispatchEvent(rej);
    expect(h.invoke).toHaveBeenCalledWith('log_ui', { level: 'error', message: 'uncaught error: boom (app.js:7)' });
    expect(h.invoke.mock.calls[1][1]).toMatchObject({ level: 'error', message: expect.stringContaining('unhandled rejection: nope') });
  });
});
