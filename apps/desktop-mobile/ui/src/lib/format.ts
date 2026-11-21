// Small formatting helpers (pure, unit-tested).

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** `YYYY-MM-DD` in the local time zone: the grouping key of the timeline. */
export function dayKey(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** "Today", "Yesterday", "Sun, 18 Oct" (with the year when it is not this year). */
export function dayLabel(key: string, now = new Date()): string {
  const [y, m, d] = key.split('-').map(Number);
  const date = new Date(y, m - 1, d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const diff = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  const base = `${DAYS[date.getDay()]}, ${d} ${MONTHS[m - 1]}`;
  return y === now.getFullYear() ? base : `${base} ${y}`;
}

export function bytes(n: number): string {
  if (n < 1000) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1000;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString('en')} ${n === 1 ? one : many}`;
}

/** "14 Oct 2023, 18:42". */
export function dateTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}, ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** "March 2025" for a month heading (`key` is any day of the month, as "2025-03-01"). */
export function monthLabel(key: string): string {
  const [y, m] = key.split('-').map(Number);
  return `${['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'][m - 1]} ${y}`;
}

/** When an album's photos were taken, as short as it can say it: "3 Jun 2025", "3–9 Jun 2025",
 *  "28 May – 9 Jun 2025", "Dec 2024 – Jan 2025". Empty without dates. */
export function dateRange(first: number | null, last: number | null): string {
  if (first === null || last === null) return '';
  const a = new Date(first), b = new Date(last);
  const [da, ma, ya] = [a.getDate(), MONTHS[a.getMonth()], a.getFullYear()];
  const [db, mb, yb] = [b.getDate(), MONTHS[b.getMonth()], b.getFullYear()];
  if (ya !== yb) return `${ma} ${ya} – ${mb} ${yb}`;
  if (ma !== mb) return `${da} ${ma} – ${db} ${mb} ${yb}`;
  if (da !== db) return `${da}–${db} ${mb} ${yb}`;
  return `${da} ${mb} ${yb}`;
}
