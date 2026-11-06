// Search helpers (S1-05): chips, suggestions and recent searches. Pure functions, unit-tested;
// the screens hold the state.
import type { SearchChip } from '../bindings/SearchChip';
import type { SearchSuggestion } from '../bindings/SearchSuggestion';

/** The query with one chip's words taken out (spans are in characters). */
export function removeChip(text: string, chip: Pick<SearchChip, 'start' | 'end'>): string {
  const chars = Array.from(text);
  const out = [...chars.slice(0, chip.start), ' ', ...chars.slice(chip.end)].join('');
  return out.replace(/\s+/g, ' ').trim();
}

/** Suggestion groups in the order they are shown. */
export const GROUPS: { kind: string; title: string; icon: string }[] = [
  { kind: 'person', title: 'People', icon: '☺' },
  { kind: 'place', title: 'Places', icon: '📍' },
  { kind: 'album', title: 'Albums', icon: '▤' },
  { kind: 'thing', title: 'Things', icon: '◇' },
  { kind: 'camera', title: 'Cameras', icon: '📷' },
  { kind: 'lens', title: 'Lenses', icon: '◎' },
];

export function chipIcon(kind: string): string {
  return { date: '📅', kind: '▶', favourite: '♥' }[kind] ?? GROUPS.find((g) => g.kind === kind)?.icon ?? '';
}

export interface Group {
  title: string;
  icon: string;
  items: SearchSuggestion[];
}

/** Suggestions grouped (People, Places, Albums, Things, Cameras, Lenses), empty groups dropped. */
export function groupSuggestions(list: SearchSuggestion[]): Group[] {
  return GROUPS.map((g) => ({ title: g.title, icon: g.icon, items: list.filter((s) => s.kind === g.kind) })).filter((g) => g.items.length > 0);
}

/** The word being typed: suggestions complete it. */
export function lastWord(text: string): string {
  const m = /(\S+)$/.exec(text);
  return m ? m[1] : '';
}

/** Replace the word being typed with a chosen name. */
export function complete(text: string, name: string): string {
  const w = lastWord(text);
  const head = w ? text.slice(0, text.length - w.length) : text.endsWith(' ') || !text ? text : `${text} `;
  return `${head}${name} `;
}

const RECENT_KEY = 'search.recent';
const RECENT_MAX = 8;

/** Recent searches on this device (never synced). Storage may be unavailable: then none. */
export function loadRecent(store: Pick<Storage, 'getItem'> | null = safeStorage()): string[] {
  try {
    const v = JSON.parse(store?.getItem(RECENT_KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((x) => typeof x === 'string').slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

/** Remember `q` first, without repeats; returns the new list. */
export function addRecent(q: string, store: Pick<Storage, 'getItem' | 'setItem'> | null = safeStorage()): string[] {
  const t = q.trim();
  const list = loadRecent(store);
  if (!t) return list;
  const next = [t, ...list.filter((x) => x.toLowerCase() !== t.toLowerCase())].slice(0, RECENT_MAX);
  try {
    store?.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Private mode or full: recent searches are a convenience.
  }
  return next;
}

export function clearRecent(store: Pick<Storage, 'removeItem'> | null = safeStorage()) {
  try {
    store?.removeItem(RECENT_KEY);
  } catch {
    // ignore
  }
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

/** Call `f` once `ms` have passed without another call. */
export function debounce<A extends unknown[]>(f: (...a: A) => void, ms: number): ((...a: A) => void) & { cancel(): void } {
  let t: ReturnType<typeof setTimeout> | undefined;
  const d = (...a: A) => {
    clearTimeout(t);
    t = setTimeout(() => f(...a), ms);
  };
  d.cancel = () => clearTimeout(t);
  return d;
}

/** Move a highlighted index through `n` items; -1 is "none" (the field itself). */
export function step(index: number, delta: number, n: number): number {
  if (n === 0) return -1;
  const next = index + delta;
  if (next < -1) return n - 1;
  if (next >= n) return -1;
  return next;
}

/** The time zone the app core should read "today" in: minutes east of UTC. */
export function tzOffset(d = new Date()): number {
  return -d.getTimezoneOffset();
}

/** `#/search/<query>` ↔ query. */
export function searchPath(q: string): string {
  return `search/${encodeURIComponent(q.trim())}`;
}

export function queryFromParam(p: string | undefined): string {
  try {
    return p ? decodeURIComponent(p) : '';
  } catch {
    return p ?? '';
  }
}

/** Under the results (S1-16): what searching by content covers so far, and who is on it. Empty
 * once every photo is covered. */
export function coverageLine(st: { total: number; content: number; meaning: boolean; eta_secs: number | null; indexers: { name: string; this_device: boolean }[] } | null): string {
  if (!st || st.total === 0) return '';
  if (!st.meaning) return 'Searching by date, place, camera, album, labels and text in photos';
  if (st.content >= st.total) return '';
  const who = st.indexers.length === 0
    ? 'no device is indexing right now'
    : `${st.indexers.slice(0, 2).map((i) => (i.this_device ? 'this device' : i.name || 'a device')).join(' and ')} indexing`;
  const eta = st.eta_secs === null ? '' : `, about ${duration(st.eta_secs)} left`;
  return `What is in the photos: ${st.content.toLocaleString()} of ${st.total.toLocaleString()} so far · ${who}${eta}`;
}

/** Settings → Search on a phone that is the only one indexing (S2-05): how long it will take, in
 * nights of charging (taken as six hours each), since the rate counts only the minutes it runs.
 * Empty when other devices share the work, or nothing is left. */
export function phoneAloneLine(st: { total: number; content: number; meaning: boolean; indexers: { this_device: boolean; photos_per_min: number }[] } | null): string {
  if (!st || !st.meaning || st.content >= st.total) return '';
  const working = st.indexers.filter((i) => i.photos_per_min > 0);
  if (working.length !== 1 || !working[0].this_device) return '';
  const minutes = (st.total - st.content) / working[0].photos_per_min;
  const nights = Math.max(1, Math.ceil(minutes / (6 * 60)));
  return `This phone is indexing on its own while it charges: ${(st.total - st.content).toLocaleString()} photos left, about ${nights === 1 ? 'one night' : `${nights} nights`}.`;
}

export function duration(secs: number): string {
  if (secs < 90) return 'a minute';
  if (secs < 3600) return `${Math.round(secs / 60)} min`;
  if (secs < 36 * 3600) return `${Math.round(secs / 3600)} h`;
  return `${Math.round(secs / 86400)} days`;
}
