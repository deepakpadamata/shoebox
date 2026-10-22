// The UI's only door to the app core. Inside Tauri it uses `invoke` and the `shoebox://` URI
// scheme; in a browser (development, review) it talks to `desktop-dev-server` over HTTP.
import type { AlbumInfo } from '../bindings/AlbumInfo';
import type { AppEvent } from '../bindings/AppEvent';
import type { AppState } from '../bindings/AppState';
import type { CreatedLibrary } from '../bindings/CreatedLibrary';
import type { DeviceInfo } from '../bindings/DeviceInfo';
import type { FreeUpReport } from '../bindings/FreeUpReport';
import type { Health } from '../bindings/Health';
import type { ImportSummary } from '../bindings/ImportSummary';
import type { LibraryCounts } from '../bindings/LibraryCounts';
import type { PageCursor } from '../bindings/PageCursor';
import type { PairStart } from '../bindings/PairStart';
import type { PhotoDetail } from '../bindings/PhotoDetail';
import type { PhotoPage } from '../bindings/PhotoPage';
import type { PhotoSummary } from '../bindings/PhotoSummary';
import type { Settings } from '../bindings/Settings';

export type {
  AlbumInfo, AppEvent, AppState, CreatedLibrary, DeviceInfo, FreeUpReport, Health, ImportSummary,
  LibraryCounts, PageCursor, PairStart, PhotoDetail, PhotoPage, PhotoSummary, Settings,
};

export type BlobKind = 'thumb' | 'preview' | 'original';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** An error from the app core, already phrased for the user. */
export class AppError extends Error {}

async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) {
    const { invoke } = await import('@tauri-apps/api/core');
    try {
      return (await invoke('command', { cmd, args })) as T;
    } catch (e) {
      throw new AppError(String(e));
    }
  }
  const res = await fetch(`/api/${cmd}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(args),
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new AppError(body.error ?? `request failed (${res.status})`);
  return body as T;
}

/** URL of one rendition of a photo; never base64, served from the blob store. */
export function blobUrl(id: string, kind: BlobKind): string {
  if (inTauri) {
    // macOS/Linux use `shoebox://localhost/…`; Windows and Android rewrite custom schemes to http.
    const windows = navigator.userAgent.includes('Windows') || navigator.userAgent.includes('Android');
    const base = windows ? 'http://shoebox.localhost' : 'shoebox://localhost';
    return `${base}/blob/${id}?kind=${kind}`;
  }
  return `/blob/${id}?kind=${kind}`;
}

/** Subscribe to app events; returns the unsubscribe function. */
export function onAppEvent(handler: (e: AppEvent) => void): () => void {
  if (inTauri) {
    let stop: (() => void) | undefined;
    let cancelled = false;
    import('@tauri-apps/api/event').then(({ listen }) =>
      listen<AppEvent>('app-event', (e) => handler(e.payload)).then((un) => {
        if (cancelled) un();
        else stop = un;
      }),
    );
    return () => {
      cancelled = true;
      stop?.();
    };
  }
  const es = new EventSource('/events');
  es.onmessage = (m) => handler(JSON.parse(m.data) as AppEvent);
  return () => es.close();
}

export const api = {
  appState: () => call<AppState>('app_state'),
  createLibrary: (device_name: string, role: string) => call<CreatedLibrary>('create_library', { device_name, role }),
  join: (ticket: string, device_name: string) => call<{}>('join', { ticket, device_name }),
  recover: (link: string, phrase: string, device_name: string) => call<{}>('recover', { link, phrase, device_name }),
  confirmPairing: (accept: boolean) => call<{}>('confirm_pairing', { accept }),
  pairStart: (role: string) => call<PairStart>('pair_start', { role }),
  markRecoveryKitSaved: () => call<{}>('mark_recovery_kit_saved'),
  libraryPage: (cursor: PageCursor | null, limit = 500) => call<PhotoPage>('library_page', { cursor, limit }),
  counts: () => call<LibraryCounts>('counts'),
  photoDetail: (id: string) => call<PhotoDetail>('photo_detail', { id }),
  setFavorite: (ids: string[], favorite: boolean) => call<{}>('set_favorite', { ids, favorite }),
  delete: (ids: string[]) => call<{}>('delete', { ids }),
  restore: (ids: string[]) => call<{}>('restore', { ids }),
  recentlyDeleted: () => call<PhotoSummary[]>('recently_deleted'),
  importPaths: (paths: string[]) => call<ImportSummary>('import_paths', { paths }),
  pinOriginal: (id: string, pin: boolean) => call<{}>('pin_original', { id, pin }),
  fetchNow: (id: string, kind: BlobKind) => call<{}>('fetch_now', { id, kind }),
  albums: () => call<AlbumInfo[]>('albums'),
  createAlbum: (name: string) => call<AlbumInfo>('create_album', { name }),
  albumAdd: (album: string, ids: string[]) => call<{}>('album_add', { album, ids }),
  albumRemove: (album: string, ids: string[]) => call<{}>('album_remove', { album, ids }),
  albumPage: (album: string, cursor: PageCursor | null, limit = 500) => call<PhotoPage>('album_page', { album, cursor, limit }),
  devices: () => call<DeviceInfo[]>('devices'),
  revokeDevice: (id: string) => call<{}>('revoke_device', { id }),
  health: () => call<Health>('health'),
  freeUpSpace: (dry_run: boolean) => call<FreeUpReport>('free_up_space', { dry_run }),
  settings: () => call<Settings>('settings'),
  setRedundancyTarget: (copies: number) => call<{}>('set_redundancy_target', { copies }),
  setRelay: (relay: string) => call<{}>('set_relay', { relay }),
  renameDevice: (name: string) => call<{}>('rename_device', { name }),
  watchFolder: (dir: string) => call<{}>('watch_folder', { dir }),
};
