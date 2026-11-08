// A tiny hash router: `#/library`, `#/album/<id>`, `#/photo/<id>` (viewer over the current list).
// Old links and tester notes keep working: Storage and Devices became the Backup page (U1-12),
// Shared albums became albums (U1-19).
const REDIRECTS: [RegExp, string][] = [
  [/^health(\/|$)/, 'backup/details'],
  [/^devices(\/|$)/, 'backup/devices'],
  [/^shared\/(.+)$/, 'album/$1'],
  [/^shared$/, 'albums'],
];

function current(): string {
  const raw = location.hash.replace(/^#\/?/, '') || 'library';
  for (const [re, to] of REDIRECTS) {
    if (re.test(raw)) {
      const next = raw.replace(re, to);
      history.replaceState(history.state, '', `#/${next}`);
      return next;
    }
  }
  return raw;
}

export const route = $state({ path: current() });

addEventListener('hashchange', () => {
  route.path = current();
});

export function go(path: string) {
  location.hash = `/${path}`;
}

/** The segments after the section: `settings/backup/x` gives `['backup', 'x']`. */
export function rest(): string[] {
  return route.path.split('/').slice(1).filter(Boolean);
}

export function section(): string {
  return route.path.split('/')[0];
}

export function param(): string | undefined {
  return route.path.split('/')[1];
}
