// Layout of the timeline: photos grouped by day into rows of square tiles, so that only the rows
// in (or near) the viewport are rendered. Pure functions, unit-tested.

import type { PhotoSummary } from './api';
import { dayKey } from './format';

export interface HeadRow {
  type: 'head';
  top: number;
  height: number;
  day: string;
  count: number;
}
export interface TileRow {
  type: 'tiles';
  top: number;
  height: number;
  /** Index range into the items array: [from, to). */
  from: number;
  to: number;
}
export type Row = HeadRow | TileRow;

export interface Layout {
  rows: Row[];
  height: number;
  cols: number;
  tile: number;
}

export interface LayoutOptions {
  width: number;
  /** Target tile edge; the real size is stretched so rows fill the width. */
  minTile: number;
  gap: number;
  headHeight: number;
}

/** Items must already be sorted newest first (as the app core returns them). */
export function buildLayout(items: PhotoSummary[], o: LayoutOptions): Layout {
  const cols = Math.max(1, Math.floor((o.width + o.gap) / (o.minTile + o.gap)));
  const tile = (o.width - o.gap * (cols - 1)) / cols;
  const rows: Row[] = [];
  let top = 0;
  let i = 0;
  while (i < items.length) {
    const day = dayKey(items[i].taken_at);
    let j = i;
    while (j < items.length && dayKey(items[j].taken_at) === day) j++;
    rows.push({ type: 'head', top, height: o.headHeight, day, count: j - i });
    top += o.headHeight;
    for (let k = i; k < j; k += cols) {
      const to = Math.min(k + cols, j);
      rows.push({ type: 'tiles', top, height: tile + o.gap, from: k, to });
      top += tile + o.gap;
    }
    i = j;
  }
  return { rows, height: top, cols, tile };
}

/** Indices of the rows intersecting `[scrollTop, scrollTop + viewport]`, widened by `overscan` px. */
export function visibleRows(layout: Layout, scrollTop: number, viewport: number, overscan = 600): [number, number] {
  const rows = layout.rows;
  if (rows.length === 0) return [0, 0];
  const lo = Math.max(0, scrollTop - overscan);
  const hi = scrollTop + viewport + overscan;
  // First row whose bottom edge is below `lo`.
  let a = 0;
  let b = rows.length;
  while (a < b) {
    const mid = (a + b) >> 1;
    if (rows[mid].top + rows[mid].height <= lo) a = mid + 1;
    else b = mid;
  }
  const start = a;
  let end = start;
  while (end < rows.length && rows[end].top < hi) end++;
  return [start, end];
}

/** The scroll offset that brings the day `key` (or the nearest earlier one) to the top. */
export function offsetOfDay(layout: Layout, key: string): number {
  let best = 0;
  for (const r of layout.rows) {
    if (r.type === 'head' && r.day >= key) best = r.top;
  }
  return best;
}
