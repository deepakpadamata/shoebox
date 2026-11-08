// The settings groups (U1-14/15): one list for the phone's home and the Mac's left pane, with the
// words the Mac's filter matches.
export interface Group {
  id: string;
  title: string;
  /** Extra words the filter finds this group by. */
  keywords: string;
}

export const GROUPS: Group[] = [
  { id: 'camera', title: 'Camera backup', keywords: 'backup photos camera roll wi-fi wifi charging folders keep recent favourites favorites import photos library' },
  { id: 'you', title: 'You & this device', keywords: 'name profile device role rename' },
  { id: 'search', title: 'Search & People', keywords: 'search index indexing faces people models' },
  { id: 'appearance', title: 'Appearance', keywords: 'theme dark light system text size font' },
  { id: 'recovery', title: 'Recovery kit', keywords: 'recovery kit phrase words device link lost' },
  { id: 'advanced', title: 'Advanced', keywords: 'copies to keep relay relays connection network export watched folders diagnostics report help rebuild index replace recovery phrase leave library danger' },
  { id: 'about', title: 'About', keywords: 'about version licences licenses updates source' },
];

export const groupTitle = (id: string | undefined) => GROUPS.find((g) => g.id === id)?.title;

/** Groups whose title or keywords contain every word typed. */
export function filterGroups(q: string, groups: Group[] = GROUPS): Group[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return groups;
  return groups.filter((g) => {
    const hay = `${g.title} ${g.keywords}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}
