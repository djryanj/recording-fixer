import { api } from './api';

const text = (v: unknown): string => (v instanceof Error ? `${v.message}\n${v.stack ?? ''}` : String(v));

/** Send uncaught errors to the app's log file (on this computer only), so a bug report can include them. */
export function installErrorLogging(target: Window = window) {
  const send = (message: string) => void api.logUi('error', message).catch(() => {});
  target.addEventListener('error', (e) => send(`uncaught error: ${e.message} (${e.filename}:${e.lineno})`));
  target.addEventListener('unhandledrejection', (e) => send(`unhandled rejection: ${text(e.reason)}`));
}
