// One list of albums (U1-19): the person's own albums and the shared albums they belong to,
// where a personal album that is shared shows once, with a Shared pill, instead of in two lists.
import type { AlbumInfo } from '../bindings/AlbumInfo';
import type { SharedAlbumInfo } from '../bindings/SharedAlbumInfo';
import type { SmartAlbumInfo } from '../bindings/SmartAlbumInfo';

export interface AlbumEntry {
  /** The id in `#/album/<id>`: the personal album's, or the shared album's when there is none. */
  id: string;
  name: string;
  cover: string | null;
  /** Shared albums keep their covers in the album's own blob scope. */
  coverScope?: string;
  count: number;
  shared: boolean;
  /** A personal album (rename, set cover, delete); `false` for an album someone else owns. */
  personal: boolean;
  /** The shared album behind this entry, if any. */
  sharedId?: string;
  /** Photos other members added since it was last opened. */
  unread: number;
  /** `owner`, `contributor` or `viewer` for a shared album. */
  role?: string;
  /** A smart album: the search that finds its photos. */
  smart?: string;
  /** When its photos were taken (ms), and what it is about ("" when nothing was said). */
  firstMs: number | null;
  lastMs: number | null;
  note: string;
}

export function mergeAlbums(personal: AlbumInfo[], shared: SharedAlbumInfo[], smart: SmartAlbumInfo[] = []): AlbumEntry[] {
  const claimed = new Set<string>();
  const out: AlbumEntry[] = personal.map((a) => {
    const s = a.shared_as ? shared.find((x) => x.id === a.shared_as) : undefined;
    if (a.shared_as) claimed.add(a.shared_as);
    return { id: a.id, name: a.name, cover: a.cover, count: a.item_count, shared: !!a.shared_as, personal: true, sharedId: a.shared_as ?? undefined, unread: s?.unread ?? 0, role: s?.role, firstMs: a.first_ms ?? null, lastMs: a.last_ms ?? null, note: a.note ?? '' };
  });
  for (const m of smart) {
    out.push({ id: m.id, name: m.name, cover: null, count: 0, shared: false, personal: false, smart: m.query, unread: 0, firstMs: null, lastMs: null, note: '' });
  }
  for (const s of shared) {
    if (claimed.has(s.id)) continue;
    out.push({ id: s.id, name: s.name, cover: s.cover, coverScope: s.id, count: s.item_count, shared: true, personal: false, sharedId: s.id, unread: s.unread, role: s.role, firstMs: null, lastMs: null, note: '' });
  }
  return out.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
}

/** Which kind of album an id in `#/album/<id>` is, from the merged list. */
export const findAlbum = (list: AlbumEntry[], id: string) => list.find((a) => a.id === id || a.sharedId === id);

export type AlbumFilter = 'all' | 'mine' | 'shared';
export const filterAlbums = (list: AlbumEntry[], f: AlbumFilter) => (f === 'all' ? list : f === 'shared' ? list.filter((a) => a.shared) : list.filter((a) => a.personal || a.smart !== undefined));

/** Albums the person can delete: their own, and smart albums (shared ones are left instead). */
export const deletable = (a: AlbumEntry) => a.personal || a.smart !== undefined;
