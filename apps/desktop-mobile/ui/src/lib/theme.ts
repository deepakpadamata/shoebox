// Theme (Settings > Appearance): 'system', 'light' or 'dark', saved in localStorage['theme'] and
// applied before the app mounts so a chosen theme never flashes the system one.
export type Theme = 'system' | 'light' | 'dark';

export function savedTheme(): string {
  try {
    return localStorage.getItem('theme') ?? 'system';
  } catch {
    return 'system';
  }
}

export function applyTheme(v: string) {
  try {
    localStorage.setItem('theme', v);
  } catch {
    // Storage can be blocked; the theme still applies for this session.
  }
  paintTheme(v);
}

/** Apply without saving (launch). */
export function paintTheme(v: string) {
  if (v === 'light' || v === 'dark') document.documentElement.setAttribute('data-theme', v);
  else document.documentElement.removeAttribute('data-theme');
  syncSystemBars(v);
}

const dark = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;
let current = 'system';

/** Android: the status and navigation bar icons follow the theme (dark icons on a light page). */
function syncSystemBars(v: string) {
  current = v;
  const native = (window as unknown as { ShoeboxTheme?: { setLightBars(light: boolean): void } }).ShoeboxTheme;
  if (!native) return;
  const isDark = v === 'dark' || (v !== 'light' && !!dark?.matches);
  native.setLightBars(!isDark);
}
dark?.addEventListener?.('change', () => syncSystemBars(current));
