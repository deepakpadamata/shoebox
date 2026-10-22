// A tiny hash router: `#/library`, `#/album/<id>`, `#/photo/<id>` (viewer over the current list).
export const route = $state({ path: location.hash.replace(/^#\/?/, '') || 'library' });

addEventListener('hashchange', () => {
  route.path = location.hash.replace(/^#\/?/, '') || 'library';
});

export function go(path: string) {
  location.hash = `/${path}`;
}

export function section(): string {
  return route.path.split('/')[0];
}

export function param(): string | undefined {
  return route.path.split('/')[1];
}
