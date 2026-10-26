import { api, type UpdateInfo } from './api';

export const UPDATE_KEY = 'updateCheck.v1';
/** At most one automatic check a day: the answer rarely changes, and every check is a request to GitHub. */
export const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

interface Saved {
  enabled?: boolean;
  lastChecked?: number;
}

function read(): Saved {
  try {
    return JSON.parse(localStorage.getItem(UPDATE_KEY) ?? '{}');
  } catch {
    return {};
  }
}

function write(patch: Saved) {
  try {
    localStorage.setItem(UPDATE_KEY, JSON.stringify({ ...read(), ...patch }));
  } catch {
    /* the choice just won't be remembered */
  }
}

/** On unless the user switched it off. */
export const updateChecksEnabled = () => read().enabled !== false;
export const setUpdateChecks = (enabled: boolean) => write({ enabled });

/** True when the automatic check is on and has not run within the last day. */
export function checkIsDue(now = Date.now()): boolean {
  const s = read();
  if (s.enabled === false) return false;
  return !(typeof s.lastChecked === 'number' && now - s.lastChecked >= 0 && now - s.lastChecked < CHECK_INTERVAL_MS);
}

/** Ask for the latest release and remember when we asked (not what the answer was). */
export async function checkForUpdate(): Promise<UpdateInfo> {
  const info = await api.checkForUpdate();
  write({ lastChecked: Date.now() });
  return info;
}
