import { describe, it, expect } from 'vitest';

import { formatEta, formatRate } from './DownloadProgress';

describe('formatEta', () => {
  it('formats seconds, minutes, and hours compactly', () => {
    expect(formatEta(0)).toBe('0s');
    expect(formatEta(42)).toBe('42s');
    expect(formatEta(62)).toBe('1m 02s');
    expect(formatEta(120)).toBe('2m');
    expect(formatEta(3900)).toBe('1h 05m');
  });

  it('drops the minute suffix on whole hours', () => {
    expect(formatEta(7200)).toBe('2h');
  });

  it('rounds sub-second input to 0s', () => {
    expect(formatEta(0.4)).toBe('0s');
  });
});

describe('formatRate', () => {
  it('formats tiles/sec to one decimal', () => {
    expect(formatRate(4.3)).toBe('4.3 tiles/s');
    expect(formatRate(2)).toBe('2.0 tiles/s');
  });
});
