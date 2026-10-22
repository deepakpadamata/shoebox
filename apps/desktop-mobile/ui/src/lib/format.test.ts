import { describe, expect, it } from 'vitest';
import { bytes, dayKey, dayLabel, plural } from './format';

describe('format', () => {
  it('groups by local day', () => {
    const noon = new Date(2024, 4, 3, 12, 0).getTime();
    expect(dayKey(noon)).toBe('2024-05-03');
    expect(dayKey(new Date(2024, 4, 3, 23, 59).getTime())).toBe('2024-05-03');
    expect(dayKey(new Date(2024, 4, 4, 0, 1).getTime())).toBe('2024-05-04');
  });

  it('labels days relative to now', () => {
    const now = new Date(2024, 4, 10, 15, 0);
    expect(dayLabel('2024-05-10', now)).toBe('Today');
    expect(dayLabel('2024-05-09', now)).toBe('Yesterday');
    expect(dayLabel('2024-05-05', now)).toBe('Sun, 5 May');
    expect(dayLabel('2023-12-24', now)).toBe('Sun, 24 Dec 2023');
  });

  it('formats sizes and counts', () => {
    expect(bytes(999)).toBe('999 B');
    expect(bytes(4_200_000)).toBe('4.2 MB');
    expect(bytes(120_000_000_000)).toBe('120 GB');
    expect(plural(1, 'photo')).toBe('1 photo');
    expect(plural(1234, 'photo')).toBe('1,234 photos');
  });
});
