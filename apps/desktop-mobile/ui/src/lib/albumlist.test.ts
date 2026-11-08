import { describe, expect, it } from 'vitest';
import { filterAlbums, findAlbum, mergeAlbums } from './albumlist';

const p = (id: string, name: string, shared_as: string | null = null) => ({ id, name, cover: null, item_count: 3, shared_as });
const s = (id: string, name: string, unread = 0) => ({ id, name, role: 'owner', item_count: 5, member_count: 2, keep_originals: false, location: 'ask', unread, cover: 'c' });

describe('mergeAlbums', () => {
  it('shows a personal album that is shared once, with its unread count', () => {
    const l = mergeAlbums([p('a', 'Trip', 's1')], [s('s1', 'Trip', 4)]);
    expect(l).toHaveLength(1);
    expect(l[0]).toMatchObject({ id: 'a', shared: true, personal: true, unread: 4, sharedId: 's1' });
  });
  it('adds shared albums that have no personal album, in their own scope', () => {
    const l = mergeAlbums([p('a', 'Zoo')], [s('s9', 'Birthday')]);
    expect(l.map((x) => x.name)).toEqual(['Birthday', 'Zoo']);
    expect(l[0]).toMatchObject({ id: 's9', personal: false, shared: true, coverScope: 's9' });
  });
  it('filters and finds by either id', () => {
    const l = mergeAlbums([p('a', 'Trip', 's1'), p('b', 'Home')], [s('s1', 'Trip'), s('s9', 'Party')]);
    expect(filterAlbums(l, 'mine').map((x) => x.id).sort()).toEqual(['a', 'b']);
    expect(filterAlbums(l, 'shared')).toHaveLength(2);
    expect(findAlbum(l, 's1')?.id).toBe('a');
    expect(findAlbum(l, 'zz')).toBeUndefined();
  });
});
