import type { Target, ToolStatus } from './api';

export type Kind = 'aiff' | 'mp3' | 'aac';

export interface FormatState {
  kind: Kind;
  mp3Mode: 'cbr' | 'vbr' | 'abr';
  /** kbps for cbr/abr, LAME V level (0 best) for vbr. */
  mp3Value: number;
  id3Version: 3 | 4;
  id3v1: boolean;
  aacEncoder: string;
  aacMode: 'cbr' | 'vbr';
  /** kbps for cbr, quality level 1 (smallest) to 5 (best) for vbr. */
  aacValue: number;
}

export const MP3_CBR = [96, 128, 160, 192, 224, 256, 320];
export const MP3_ABR = [128, 160, 192, 224, 256, 320];
/** LAME VBR presets with their typical average bitrates. */
export const MP3_VBR = [
  { v: 0, kbps: 245 },
  { v: 1, kbps: 225 },
  { v: 2, kbps: 190 },
  { v: 3, kbps: 175 },
  { v: 4, kbps: 165 },
  { v: 5, kbps: 130 },
  { v: 6, kbps: 115 },
  { v: 7, kbps: 100 },
  { v: 8, kbps: 85 },
  { v: 9, kbps: 65 },
];
export const AAC_CBR = [96, 128, 160, 192, 224, 256, 320];
export const AAC_VBR = [
  { level: 1, label: '1 (smallest)' },
  { level: 2, label: '2' },
  { level: 3, label: '3' },
  { level: 4, label: '4' },
  { level: 5, label: '5 (best)' },
];

const ENCODER_LABELS: Record<string, string> = {
  apple_coreaudio: 'Apple AAC (CoreAudio)',
  aac_at: 'Apple AAC (AudioToolbox via ffmpeg)',
  libfdk_aac: 'Fraunhofer FDK AAC',
  aac: 'ffmpeg built-in AAC',
  aac_mf: 'Windows Media Foundation AAC',
};

export const encoderLabel = (name: string) => ENCODER_LABELS[name] ?? name;

export function buildTarget(s: FormatState): Target {
  switch (s.kind) {
    case 'aiff':
      return { kind: 'aiff' };
    case 'mp3':
      return { kind: 'mp3', mode: s.mp3Mode, value: s.mp3Value, id3_version: s.id3Version, id3v1: s.id3v1 };
    case 'aac':
      return { kind: 'aac', encoder: s.aacEncoder, mode: s.aacMode, value: s.aacValue };
  }
}

/** Short label for the `{quality}` template token. */
export function qualityLabel(s: FormatState): string {
  if (s.kind === 'aiff') return 'lossless';
  if (s.kind === 'mp3') return s.mp3Mode === 'vbr' ? `V${s.mp3Value}` : s.mp3Mode === 'abr' ? `ABR${s.mp3Value}` : `${s.mp3Value}k`;
  return s.aacMode === 'vbr' ? `VBR${s.aacValue}` : `${s.aacValue}k`;
}

/** Approximate average bitrate, for a size estimate. `null` for lossless. */
export function estimateKbps(s: FormatState): number | null {
  if (s.kind === 'aiff') return null;
  if (s.kind === 'mp3') return s.mp3Mode === 'vbr' ? (MP3_VBR.find((v) => v.v === s.mp3Value)?.kbps ?? 190) : s.mp3Value;
  return s.aacMode === 'vbr' ? ([96, 128, 192, 256, 320][Math.min(5, Math.max(1, s.aacValue)) - 1]) : s.aacValue;
}

export function defaultFormat(tool: ToolStatus | null): FormatState {
  return {
    kind: 'aiff',
    mp3Mode: 'cbr',
    mp3Value: 320,
    id3Version: 3,
    id3v1: false,
    aacEncoder: tool?.best_aac ?? 'aac',
    aacMode: 'cbr',
    aacValue: 256,
  };
}

/** Make a (possibly saved) choice valid for the tools that are actually available. */
export function sanitizeFormat(s: FormatState, tool: ToolStatus | null): FormatState {
  const available = tool?.aac_encoders ?? [];
  const out = { ...s };
  if (!available.includes(out.aacEncoder)) out.aacEncoder = tool?.best_aac ?? 'aac';
  if (out.kind === 'mp3' && !(tool?.ffmpeg_path && tool.mp3)) out.kind = 'aiff';
  if (out.kind === 'aac' && !(tool?.ffmpeg_path && available.length > 0)) out.kind = 'aiff';
  return out;
}

export const MAX_FADE_SECS = 60;

/** Make a fade length from a form field safe: finite, between 0 and 60 s, to a tenth of a second. */
export function clampFade(v: number | null | undefined): number {
  if (typeof v !== 'number' || !Number.isFinite(v) || v <= 0) return 0;
  return Math.round(Math.min(MAX_FADE_SECS, v) * 10) / 10;
}
