import { describe, expect, it } from 'vitest';
import type { ToolStatus } from './api';
import {
  AAC_CBR,
  buildTarget,
  clampFade,
  defaultFormat,
  encoderLabel,
  estimateKbps,
  MP3_CBR,
  MP3_VBR,
  qualityLabel,
  sanitizeFormat,
  type FormatState,
} from './exportOptions';

const tool = (over: Partial<ToolStatus> = {}): ToolStatus => ({
  ffmpeg_path: 'ffmpeg',
  ffmpeg_version: 'v',
  aac_encoders: ['aac'],
  best_aac: 'aac',
  mp3: true,
  apple: null,
  ...over,
});

const base: FormatState = {
  kind: 'mp3',
  mp3Mode: 'cbr',
  mp3Value: 320,
  id3Version: 3,
  id3v1: false,
  aacEncoder: 'aac',
  aacMode: 'cbr',
  aacValue: 256,
};

describe('buildTarget', () => {
  it('builds each target kind', () => {
    expect(buildTarget({ ...base, kind: 'aiff' })).toEqual({ kind: 'aiff' });
    expect(buildTarget(base)).toEqual({ kind: 'mp3', mode: 'cbr', value: 320, id3_version: 3, id3v1: false });
    expect(buildTarget({ ...base, mp3Mode: 'vbr', mp3Value: 0, id3Version: 4, id3v1: true })).toEqual({
      kind: 'mp3',
      mode: 'vbr',
      value: 0,
      id3_version: 4,
      id3v1: true,
    });
    expect(buildTarget({ ...base, kind: 'aac', aacMode: 'vbr', aacValue: 4 })).toEqual({
      kind: 'aac',
      encoder: 'aac',
      mode: 'vbr',
      value: 4,
    });
  });
});

describe('qualityLabel', () => {
  it('is short and filename friendly', () => {
    expect(qualityLabel(base)).toBe('320k');
    expect(qualityLabel({ ...base, mp3Mode: 'vbr', mp3Value: 2 })).toBe('V2');
    expect(qualityLabel({ ...base, mp3Mode: 'abr', mp3Value: 192 })).toBe('ABR192');
    expect(qualityLabel({ ...base, kind: 'aac' })).toBe('256k');
    expect(qualityLabel({ ...base, kind: 'aac', aacMode: 'vbr', aacValue: 5 })).toBe('VBR5');
    expect(qualityLabel({ ...base, kind: 'aiff' })).toBe('lossless');
  });
});

describe('presets', () => {
  it('offer the usual MP3 bitrates and all ten LAME VBR levels', () => {
    expect(MP3_CBR).toContain(320);
    expect(MP3_CBR).toContain(128);
    expect(MP3_VBR.map((v) => v.v)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(AAC_CBR).toContain(256);
  });
});

describe('defaultFormat / sanitizeFormat', () => {
  it('defaults to the best available AAC encoder and a high bitrate', () => {
    const f = defaultFormat(tool({ aac_encoders: ['apple_coreaudio', 'aac'], best_aac: 'apple_coreaudio' }));
    expect(f.aacEncoder).toBe('apple_coreaudio');
    expect(f.mp3Value).toBe(320);
    expect(f.kind).toBe('aiff');
  });

  it('falls back when a saved encoder is no longer available', () => {
    const s = sanitizeFormat({ ...base, kind: 'aac', aacEncoder: 'libfdk_aac' }, tool());
    expect(s.aacEncoder).toBe('aac');
  });

  it('drops to AIFF when ffmpeg cannot do the chosen format', () => {
    expect(sanitizeFormat({ ...base, kind: 'mp3' }, tool({ mp3: false })).kind).toBe('aiff');
    expect(sanitizeFormat({ ...base, kind: 'aac' }, tool({ aac_encoders: [], best_aac: null })).kind).toBe('aiff');
    expect(sanitizeFormat({ ...base, kind: 'mp3' }, null).kind).toBe('aiff');
  });

  it('keeps a valid saved choice', () => {
    expect(sanitizeFormat(base, tool())).toEqual(base);
  });
});

describe('estimateKbps', () => {
  it('uses the chosen bitrate, or a typical rate for VBR', () => {
    expect(estimateKbps(base)).toBe(320);
    expect(estimateKbps({ ...base, mp3Mode: 'abr', mp3Value: 192 })).toBe(192);
    expect(estimateKbps({ ...base, mp3Mode: 'vbr', mp3Value: 0 })).toBe(245);
    expect(estimateKbps({ ...base, mp3Mode: 'vbr', mp3Value: 9 })).toBe(65);
    expect(estimateKbps({ ...base, kind: 'aac', aacValue: 192 })).toBe(192);
    expect(estimateKbps({ ...base, kind: 'aac', aacMode: 'vbr', aacValue: 5 })).toBe(320);
    expect(estimateKbps({ ...base, kind: 'aiff' })).toBeNull();
  });
});

describe('encoderLabel', () => {
  it('uses friendly names', () => {
    expect(encoderLabel('apple_coreaudio')).toContain('Apple');
    expect(encoderLabel('aac')).toContain('ffmpeg');
    expect(encoderLabel('mystery')).toBe('mystery');
  });
});

describe('clampFade', () => {
  it('keeps sensible values, rounded to a tenth', () => {
    expect(clampFade(0)).toBe(0);
    expect(clampFade(2)).toBe(2);
    expect(clampFade(2.54)).toBe(2.5);
    expect(clampFade(0.04)).toBe(0);
    expect(clampFade(0.06)).toBe(0.1);
  });
  it('turns nonsense into no fade and caps long ones at a minute', () => {
    for (const bad of [-5, NaN, Infinity, null, undefined]) expect(clampFade(bad as number)).toBe(0);
    expect(clampFade(600)).toBe(60);
  });
});