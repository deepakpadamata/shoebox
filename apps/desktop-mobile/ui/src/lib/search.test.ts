import { describe, expect, it, vi } from 'vitest';
import { addRecent, chipIcon, clearRecent, complete, debounce, groupSuggestions, lastWord, loadRecent, queryFromParam, removeChip, searchPath, step, coverageLine } from './search';

function memory() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  };
}

describe('chips', () => {
  it('removes a chip by its character span', () => {
    expect(removeChip('dogs in Lisbon 2019', { start: 8, end: 14 })).toBe('dogs in 2019');
    expect(removeChip('2019', { start: 0, end: 4 })).toBe('');
    // Spans count characters, not UTF-16 units or bytes.
    expect(removeChip('São Paulo 📷 2019', { start: 12, end: 16 })).toBe('São Paulo 📷');
  });

  it('has an icon per kind', () => {
    expect(chipIcon('date')).toBe('📅');
    expect(chipIcon('place')).toBe('📍');
    expect(chipIcon('nonsense')).toBe('');
  });
});

describe('suggestions', () => {
  it('groups in display order and drops empty groups', () => {
    const g = groupSuggestions([
      { kind: 'camera', name: 'Pixel 8' },
      { kind: 'place', name: 'Lisbon' },
      { kind: 'person', name: 'Ana' },
      { kind: 'place', name: 'Lagos' },
    ]);
    expect(g.map((x) => x.title)).toEqual(['People', 'Places', 'Cameras']);
    expect(g[1].items.map((x) => x.name)).toEqual(['Lisbon', 'Lagos']);
  });

  it('completes the word being typed', () => {
    expect(lastWord('dogs in lis')).toBe('lis');
    expect(lastWord('dogs ')).toBe('');
    expect(complete('dogs in lis', 'Lisbon')).toBe('dogs in Lisbon ');
    expect(complete('dogs ', 'Lisbon')).toBe('dogs Lisbon ');
    expect(complete('', 'Lisbon')).toBe('Lisbon ');
  });

  it('moves through suggestions and back to the field', () => {
    expect(step(-1, 1, 3)).toBe(0);
    expect(step(2, 1, 3)).toBe(-1);
    expect(step(-1, -1, 3)).toBe(2);
    expect(step(0, -1, 3)).toBe(-1);
    expect(step(-1, 1, 0)).toBe(-1);
  });
});

describe('recent searches', () => {
  it('keeps the newest first, without repeats, at most eight', () => {
    const s = memory();
    for (const q of ['a', 'b', 'A', ' ', 'c', 'd', 'e', 'f', 'g', 'h', 'i']) addRecent(q, s);
    expect(loadRecent(s)).toEqual(['i', 'h', 'g', 'f', 'e', 'd', 'c', 'A']);
    clearRecent(s);
    expect(loadRecent(s)).toEqual([]);
  });

  it('survives broken or missing storage', () => {
    expect(loadRecent(null)).toEqual([]);
    expect(loadRecent({ getItem: () => '{not json' })).toEqual([]);
    const throwing = { getItem: () => null, setItem: () => { throw new Error('quota'); } };
    expect(addRecent('x', throwing)).toEqual(['x']);
  });
});

describe('debounce', () => {
  it('runs once after the pause', () => {
    vi.useFakeTimers();
    const f = vi.fn();
    const d = debounce(f, 120);
    d(1);
    d(2);
    vi.advanceTimersByTime(119);
    expect(f).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(f).toHaveBeenCalledWith(2);
    d(3);
    d.cancel();
    vi.advanceTimersByTime(500);
    expect(f).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
});

describe('routes', () => {
  it('round-trips a query through the hash', () => {
    const q = 'Lisbon 2019 / 50% off';
    const p = searchPath(q).split('/').slice(1).join('/');
    expect(queryFromParam(p)).toBe(q);
    expect(queryFromParam('%E0%A4%A')).toBe('%E0%A4%A');
    expect(queryFromParam(undefined)).toBe('');
  });
});

describe('coverageLine', () => {
  const st = (o: Partial<Parameters<typeof coverageLine>[0] & object>) => ({
    total: 1000, content: 1000, meaning: true, eta_secs: null, indexers: [], ...o,
  });
  it('is hidden once every photo is covered', () => {
    expect(coverageLine(st({}))).toBe('');
    expect(coverageLine(null)).toBe('');
  });
  it('says how far indexing is and who is doing it', () => {
    expect(coverageLine(st({ content: 250, eta_secs: 7200, indexers: [{ name: 'Keeper', this_device: false }] })))
      .toBe('What is in the photos: 250 of 1,000 so far · Keeper indexing, about 2 h left');
    expect(coverageLine(st({ content: 0 }))).toContain('no device is indexing right now');
  });
  it('says what a device without the text model searches by', () => {
    expect(coverageLine(st({ meaning: false }))).toContain('labels and text in photos');
  });
});
