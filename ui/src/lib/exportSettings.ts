import type { Metadata } from './api';
import type { FormatState } from './exportOptions';

/** What the export editor and the Settings window both remember between sessions. */
export interface SavedExport {
  format: FormatState;
  nameTemplate: string;
  tags: Metadata;
  outputDir?: string;
  fadeIn?: number;
  fadeOut?: number;
}

export const SETTINGS_KEY = 'exportSettings.v1';

export function loadSavedExport(): Partial<SavedExport> {
  try {
    return JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}');
  } catch {
    return {};
  }
}

/** Merge `patch` into what is saved, so the Settings window never wipes the editor's other fields. */
export function patchSavedExport(patch: Partial<SavedExport>) {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...loadSavedExport(), ...patch }));
  } catch {
    /* settings just won't be remembered */
  }
}
