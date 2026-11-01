import type { LoudnessJob, Target, ToolStatus } from './api';

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

export type LevelingChoice = 'off' | 'gentle' | 'medium' | 'strong';

/** The loudness section of the export options. Numbers are `null` while their field is empty. */
export interface LoudnessState {
  mode: 'off' | 'lufs' | 'peak';
  /** Target integrated loudness in LUFS. */
  lufs: number | null;
  /** True-peak limit in dBTP. */
  ceiling: number | null;
  leveling: LevelingChoice;
  /** Play the editor's preview through the processing, to hear what the export will do. */
  preview: boolean;
}

export const LEVELING_CHOICES: { value: LevelingChoice; label: string }[] = [
  { value: 'off', label: 'None: one steady gain for the whole set' },
  { value: 'gentle', label: 'Gentle' },
  { value: 'medium', label: 'Medium (recommended)' },
  { value: 'strong', label: 'Strong' },
];

/** Common targets, as buttons next to the number. */
export const LUFS_PRESETS = [
  { lufs: -14, label: 'Streaming −14' },
  { lufs: -16, label: 'Quieter −16' },
  { lufs: -11, label: 'Louder −11' },
];

export const defaultLoudness = (): LoudnessState => ({ mode: 'off', lufs: -14, ceiling: -1, leveling: 'medium', preview: true });

/** Fill in anything a saved value lacks, and drop anything that is not valid. */
export function sanitizeLoudness(s: Partial<LoudnessState> | undefined): LoudnessState {
  const d = defaultLoudness();
  if (!s) return d;
  return {
    mode: s.mode === 'lufs' || s.mode === 'peak' ? s.mode : 'off',
    lufs: typeof s.lufs === 'number' && Number.isFinite(s.lufs) ? s.lufs : d.lufs,
    ceiling: typeof s.ceiling === 'number' && Number.isFinite(s.ceiling) ? s.ceiling : d.ceiling,
    leveling: LEVELING_CHOICES.some((c) => c.value === s.leveling) ? (s.leveling as LevelingChoice) : d.leveling,
    preview: s.preview !== false,
  };
}

const round1 = (v: number) => Math.round(v * 10) / 10;

/** A target loudness from a form field: finite, between -40 and -3 LUFS. */
export function clampLufs(v: number | null | undefined): number {
  return typeof v === 'number' && Number.isFinite(v) ? round1(Math.min(-3, Math.max(-40, v))) : -14;
}

/** A true-peak limit from a form field: finite, between -20 and 0 dBTP. */
export function clampCeiling(v: number | null | undefined): number {
  return typeof v === 'number' && Number.isFinite(v) ? round1(Math.min(0, Math.max(-20, v))) : -1;
}

export function buildLoudness(s: LoudnessState): LoudnessJob | null {
  if (s.mode === 'off') return null;
  return { mode: s.mode, lufs: clampLufs(s.lufs), ceiling_db: clampCeiling(s.ceiling), leveling: s.mode === 'peak' ? 'off' : s.leveling };
}

export const MAX_FADE_SECS = 60;

/** Make a fade length from a form field safe: finite, between 0 and 60 s, to a tenth of a second. */
export function clampFade(v: number | null | undefined): number {
  if (typeof v !== 'number' || !Number.isFinite(v) || v <= 0) return 0;
  return Math.round(Math.min(MAX_FADE_SECS, v) * 10) / 10;
}
