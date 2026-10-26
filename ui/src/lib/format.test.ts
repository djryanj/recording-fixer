import { describe, expect, it } from 'vitest';
import { describeFormat, formatDuration, formatPrecise, formatRate, formatSize, meterPercent, parseTime } from './format';

describe('formatDuration', () => {
  it('uses m:ss under an hour and h:mm:ss above', () => {
    expect(formatDuration(65)).toBe('1:05');
    expect(formatDuration(1145)).toBe('19:05');
    expect(formatDuration(10800)).toBe('3:00:00');
    expect(formatDuration(3725.4)).toBe('1:02:05');
  });
  it('handles missing and zero', () => {
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(0)).toBe('0:00');
  });
});

describe('formatSize', () => {
  it('scales units', () => {
    expect(formatSize(512)).toBe('512 B');
    expect(formatSize(220_484_662)).toBe('210 MB');
    expect(formatSize(1_905_124_985)).toBe('1.77 GB');
  });
});

describe('formatRate / describeFormat', () => {
  it('prints kHz without trailing zeros', () => {
    expect(formatRate(44100)).toBe('44.1 kHz');
    expect(formatRate(48000)).toBe('48 kHz');
  });
  it('describes a whole format', () => {
    expect(describeFormat({ sample_rate: 44100, bits: 16, channels: 2 })).toBe('44.1 kHz · 16-bit · stereo');
    expect(describeFormat({ sample_rate: 48000, bits: 24, channels: 1 })).toBe('48 kHz · 24-bit · mono');
    expect(describeFormat({ sample_rate: null, bits: null, channels: null })).toBe('—');
  });
});

describe('meterPercent', () => {
  it('is 0 for silence and 100 at full scale', () => {
    expect(meterPercent(0)).toBe(0);
    expect(meterPercent(-1)).toBe(0);
    expect(meterPercent(NaN)).toBe(0);
    expect(meterPercent(1)).toBe(100);
    expect(meterPercent(5)).toBe(100);
  });
  it('is logarithmic: -6 dB is a bit under the top, -54 dB is empty', () => {
    expect(meterPercent(0.5)).toBeCloseTo(100 - 6.02 * (100 / 54), 1);
    expect(meterPercent(10 ** (-54 / 20))).toBeCloseTo(0, 5);
    expect(meterPercent(0.001)).toBe(0);
    expect(meterPercent(0.1)).toBeGreaterThan(meterPercent(0.05));
  });
});

describe('formatPrecise', () => {
  it('shows tenths for trim editing', () => {
    expect(formatPrecise(0)).toBe('0:00.0');
    expect(formatPrecise(65.34)).toBe('1:05.3');
    expect(formatPrecise(3725.96)).toBe('1:02:05.9');
  });
  it('round-trips through parseTime to within a tenth', () => {
    for (const s of [0, 12.3, 599.9, 4000.5]) expect(Math.abs((parseTime(formatPrecise(s)) ?? -1) - s)).toBeLessThan(0.1);
  });
});

describe('parseTime', () => {
  it('parses seconds, m:ss and h:mm:ss with fractions', () => {
    expect(parseTime('90')).toBe(90);
    expect(parseTime('1:30')).toBe(90);
    expect(parseTime('1:02:03.5')).toBe(3723.5);
    expect(parseTime(' 0:10 ')).toBe(10);
  });
  it('rejects junk and negatives', () => {
    expect(parseTime('')).toBeNull();
    expect(parseTime('abc')).toBeNull();
    expect(parseTime('-5')).toBeNull();
    expect(parseTime('1:2:3:4')).toBeNull();
    expect(parseTime('1:75')).toBeNull();
  });
});
