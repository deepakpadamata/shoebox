// Native file and folder pickers (desktop shell). In a browser there is no picker: callers fall back
// to a typed path.
export const canPick = () => '__TAURI_INTERNALS__' in window && !/Android|iPhone|iPad/i.test(navigator.userAgent);

const MEDIA = ['jpg', 'jpeg', 'png', 'heic', 'heif', 'dng', 'cr2', 'cr3', 'nef', 'arw', 'raf', 'orf', 'rw2', 'tif', 'tiff', 'gif', 'webp', 'mov', 'mp4', 'm4v'];

/** Photos and videos chosen by the user; empty if cancelled. */
export async function pickFiles(): Promise<string[]> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const r = await open({ multiple: true, directory: false, filters: [{ name: 'Photos and videos', extensions: MEDIA }] });
  return r ?? [];
}

/** One folder chosen by the user, or null. */
export async function pickFolder(title?: string): Promise<string | null> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const r = await open({ multiple: false, directory: true, title });
  return r ?? null;
}
