import { describe, expect, it } from 'vitest';
import { filterGroups, GROUPS, groupTitle } from './groups';

describe('settings filter', () => {
  it('finds relay, theme and export', () => {
    expect(filterGroups('relay').map((g) => g.id)).toEqual(['advanced']);
    expect(filterGroups('theme').map((g) => g.id)).toEqual(['appearance']);
    expect(filterGroups('export').map((g) => g.id)).toEqual(['advanced']);
  });
  it('shows everything for an empty filter and nothing for nonsense', () => {
    expect(filterGroups('  ')).toHaveLength(GROUPS.length);
    expect(filterGroups('zzzz')).toEqual([]);
  });
  it('needs every word', () => {
    expect(filterGroups('recovery phrase').map((g) => g.id).sort()).toEqual(['advanced', 'recovery']);
    expect(filterGroups('recovery theme')).toEqual([]);
  });
  it('titles a group by id', () => {
    expect(groupTitle('advanced')).toBe('Advanced');
    expect(groupTitle('x')).toBeUndefined();
  });
});
