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
  /** Index of the day's first item. */
  from: number;
  /** A packed head (U1-18): consecutive quiet days under one heading, newest first. */
  days?: { day: string; count: number }[];
  /** A month heading (zoomed all the way out): `day` is the month's first day. */
  month?: boolean;
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
  /** Group by day under date headers (the default); off for results ordered by relevance. */
  days?: boolean;
  /** Combine consecutive days with few photos under one heading. */
  pack?: boolean;
  /** `month`: one heading per month, for the smallest tiles. */
  group?: 'day' | 'month';
}

/** A day is quiet when it would leave most of its first row empty. */
export const isQuiet = (count: number, cols: number) => count <= Math.max(2, Math.floor(cols / 2));

interface DayGroup { day: string; from: number; to: number }

function dayGroups(items: PhotoSummary[]): DayGroup[] {
  const out: DayGroup[] = [];
  for (let i = 0; i < items.length; ) {
    const day = dayKey(items[i].taken_at);
    let j = i;
    while (j < items.length && dayKey(items[j].taken_at) === day) j++;
    out.push({ day, from: i, to: j });
    i = j;
  }
  return out;
}

/** What a packed heading says: "Mon 3 · Tue 4 · Wed 5" (the month once it changes). */
export function packedLabel(days: { day: string }[]): string {
  const W = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const M = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  let lastMonth = '';
  return [...days].reverse().map(({ day }) => {
    const [y, m, d] = day.split('-').map(Number);
    const ym = `${y}-${m}`;
    const base = `${W[new Date(y, m - 1, d).getDay()]} ${d}`;
    const out = ym === lastMonth ? base : `${base} ${M[m - 1]}`;
    lastMonth = ym;
    return out;
  }).join(' · ');
}

/** Items must already be sorted newest first (as the app core returns them). */
export function buildLayout(items: PhotoSummary[], o: LayoutOptions): Layout {
  const cols = Math.max(1, Math.floor((o.width + o.gap) / (o.minTile + o.gap)));
  const tile = (o.width - o.gap * (cols - 1)) / cols;
  const rows: Row[] = [];
  let top = 0;
  let i = 0;
  if (o.days === false) {
    for (let k = 0; k < items.length; k += cols) {
      rows.push({ type: 'tiles', top, height: tile + o.gap, from: k, to: Math.min(k + cols, items.length) });
      top += tile + o.gap;
    }
    return { rows, height: top, cols, tile };
  }
  // The unit under one heading: a day, several quiet days, or a month.
  let blocks: { day: string; from: number; to: number; days?: { day: string; count: number }[]; month?: boolean }[] = [];
  const groups = dayGroups(items);
  if (o.group === 'month') {
    for (const g of groups) {
      const last = blocks[blocks.length - 1];
      const ym = g.day.slice(0, 7);
      if (last && last.day.slice(0, 7) === ym) last.to = g.to;
      else blocks.push({ day: `${ym}-01`, from: g.from, to: g.to, month: true });
    }
  } else if (o.pack) {
    for (let k = 0; k < groups.length; ) {
      const g = groups[k];
      if (!isQuiet(g.to - g.from, cols)) {
        blocks.push({ day: g.day, from: g.from, to: g.to });
        k++;
        continue;
      }
      // Quiet days next to each other, while they still fit in two rows.
      let m = k;
      let n = 0;
      while (m < groups.length && isQuiet(groups[m].to - groups[m].from, cols) && n + (groups[m].to - groups[m].from) <= cols * 2 && m - k < 7) {
        n += groups[m].to - groups[m].from;
        m++;
      }
      if (m - k < 2) {
        blocks.push({ day: g.day, from: g.from, to: g.to });
        k++;
      } else {
        blocks.push({ day: g.day, from: g.from, to: groups[m - 1].to, days: groups.slice(k, m).map((x) => ({ day: x.day, count: x.to - x.from })) });
        k = m;
      }
    }
  } else {
    blocks = groups.map((g) => ({ day: g.day, from: g.from, to: g.to }));
  }
  for (const b of blocks) {
    rows.push({ type: 'head', top, height: o.headHeight, day: b.day, count: b.to - b.from, from: b.from, ...(b.days ? { days: b.days } : {}), ...(b.month ? { month: true } : {}) });
    top += o.headHeight;
    for (let k = b.from; k < b.to; k += cols) {
      const to = Math.min(k + cols, b.to);
      rows.push({ type: 'tiles', top, height: tile + o.gap, from: k, to });
      top += tile + o.gap;
    }
  }
  void i;
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

/** The day shown at the scroll offset `y`: that of the last day heading at or above it. */
export function dayAt(layout: Layout, y: number): string | null {
  let day: string | null = null;
  for (const r of layout.rows) {
    if (r.top > y) break;
    if (r.type === 'head') day = r.day;
  }
  return day ?? (layout.rows[0]?.type === 'head' ? layout.rows[0].day : null);
}

export interface DateMark {
  /** Scroll offset of the first heading of the month. */
  top: number;
  year: number;
  /** 1–12. */
  month: number;
}

/** One mark per month present in the timeline, newest first, for the date scroller. */
export function dateMarks(layout: Layout): DateMark[] {
  const out: DateMark[] = [];
  let last = '';
  for (const r of layout.rows) {
    if (r.type !== 'head') continue;
    const ym = r.day.slice(0, 7);
    if (ym === last) continue;
    last = ym;
    out.push({ top: r.top, year: Number(ym.slice(0, 4)), month: Number(ym.slice(5, 7)) });
  }
  return out;
}
