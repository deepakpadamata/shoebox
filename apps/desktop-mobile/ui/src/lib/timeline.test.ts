import { describe, expect, it } from 'vitest';
import type { PhotoSummary } from './api';
import { buildLayout, offsetOfDay, visibleRows } from './timeline';

function photo(i: number, day: number): PhotoSummary {
  return {
    id: String(i).padStart(64, '0'),
    taken_at: new Date(2024, 4, day, 12, 0, 0).getTime() - i,
    favorite: false, is_video: false, copies: 1, has_thumb: true, has_poster: false, has_original: true, width: 4, height: 3,
  };
}

const opts = { width: 420, minTile: 100, gap: 2, headHeight: 40 };

describe('timeline layout', () => {
  it('fits columns to the width and groups by day', () => {
    // 5 photos on day 10, 3 on day 9.
    const items = [...Array(5)].map((_, i) => photo(i, 10)).concat([...Array(3)].map((_, i) => photo(10 + i, 9)));
    const l = buildLayout(items, opts);
    expect(l.cols).toBe(4);
    expect(l.tile).toBeCloseTo((420 - 6) / 4);
    expect(l.rows.map((r) => r.type)).toEqual(['head', 'tiles', 'tiles', 'head', 'tiles']);
    const heads = l.rows.filter((r) => r.type === 'head');
    expect(heads.map((h) => (h.type === 'head' ? h.count : 0))).toEqual([5, 3]);
    const tiles = l.rows.filter((r) => r.type === 'tiles');
    expect(tiles.map((r) => (r.type === 'tiles' ? [r.from, r.to] : []))).toEqual([[0, 4], [4, 5], [5, 8]]);
    // Rows are contiguous and the total height is their sum.
    let top = 0;
    for (const r of l.rows) {
      expect(r.top).toBeCloseTo(top);
      top += r.height;
    }
    expect(l.height).toBeCloseTo(top);
  });

  it('handles empty and narrow layouts', () => {
    expect(buildLayout([], opts).height).toBe(0);
    const l = buildLayout([photo(0, 1)], { ...opts, width: 50 });
    expect(l.cols).toBe(1);
  });

  it('renders only the rows near the viewport', () => {
    const items = [...Array(10_000)].map((_, i) => photo(i, 28 - Math.floor(i / 100) % 28));
    const l = buildLayout(items, opts);
    const [a, b] = visibleRows(l, 0, 800, 0);
    expect(a).toBe(0);
    expect(b).toBeLessThan(10);
    const mid = l.height / 2;
    const [c, d] = visibleRows(l, mid, 800, 200);
    expect(d - c).toBeLessThan(20);
    expect(l.rows[c].top + l.rows[c].height).toBeGreaterThan(mid - 200);
    expect(l.rows[d - 1].top).toBeLessThan(mid + 800 + 200);
    const [e, f] = visibleRows(l, l.height + 1000, 800, 0);
    expect(e).toBe(l.rows.length);
    expect(f).toBe(l.rows.length);
  });

  it('finds a day to jump to', () => {
    const items = [photo(0, 10), photo(1, 9), photo(2, 8)];
    const l = buildLayout(items, opts);
    const heads = l.rows.filter((r) => r.type === 'head');
    expect(offsetOfDay(l, '2024-05-09')).toBe(heads[1].top);
  });
});
