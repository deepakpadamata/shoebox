// Text size (Settings > Appearance): the whole page scales, which also scales every fixed-pixel
// font. Some webviews ignore the system font size, so the app offers its own.
export const TEXT_SIZES: Record<string, number> = { small: 0.92, default: 1, large: 1.15, xlarge: 1.3 };

export function applyTextSize(key: string) {
  const z = TEXT_SIZES[key] ?? 1;
  (document.documentElement.style as unknown as { zoom: string }).zoom = z === 1 ? '' : String(z);
}

export function savedTextSize(): string {
  try {
    return localStorage.getItem('textSize') ?? 'default';
  } catch {
    return 'default';
  }
}
