import { describe, expect, it } from 'vitest';
import { bytes, dateRange, dayKey, dayLabel, plural } from './format';

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
  it("says an album's dates as briefly as it can", () => {
    const d = (y: number, m: number, day: number) => new Date(y, m - 1, day, 12).getTime();
    expect(dateRange(null, null)).toBe('');
    expect(dateRange(d(2025, 6, 3), d(2025, 6, 3))).toBe('3 Jun 2025');
    expect(dateRange(d(2025, 6, 3), d(2025, 6, 9))).toBe('3–9 Jun 2025');
    expect(dateRange(d(2025, 5, 28), d(2025, 6, 9))).toBe('28 May – 9 Jun 2025');
    expect(dateRange(d(2024, 12, 30), d(2025, 1, 2))).toBe('Dec 2024 – Jan 2025');
  });
});
