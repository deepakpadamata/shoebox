// The UI's only door to the app core. Inside Tauri it uses `invoke` and the `shoebox://` URI
// scheme; in a browser (development, review) it talks to `desktop-dev-server` over HTTP.
import type { DiagnosticsBundle } from '../bindings/DiagnosticsBundle';
import type { AddCheck } from '../bindings/AddCheck';
import type { AddResult } from '../bindings/AddResult';
import type { FeedEntry } from '../bindings/FeedEntry';
import type { InviteOut } from '../bindings/InviteOut';
import type { SharedAlbumInfo } from '../bindings/SharedAlbumInfo';
import type { SharedItemInfo } from '../bindings/SharedItemInfo';
import type { SharedItemsPage } from '../bindings/SharedItemsPage';
import type { SharedMemberInfo } from '../bindings/SharedMemberInfo';
import type { AlbumInfo } from '../bindings/AlbumInfo';
import type { AppEvent } from '../bindings/AppEvent';
import type { AppState } from '../bindings/AppState';
import type { BackupPolicy } from '../bindings/BackupPolicy';
import type { BackupReport } from '../bindings/BackupReport';
import type { BackupStatus } from '../bindings/BackupStatus';
import type { CreatedLibrary } from '../bindings/CreatedLibrary';
import type { DeviceInfo } from '../bindings/DeviceInfo';
import type { DeviceStorageInfo } from '../bindings/DeviceStorageInfo';
import type { ExportReport } from '../bindings/ExportReport';
import type { FreeUpReport } from '../bindings/FreeUpReport';
import type { Health } from '../bindings/Health';
import type { ImportSummary } from '../bindings/ImportSummary';
import type { LibraryCounts } from '../bindings/LibraryCounts';
import type { PageCursor } from '../bindings/PageCursor';
import type { PairStart } from '../bindings/PairStart';
import type { PhotoDetail } from '../bindings/PhotoDetail';
import type { PhotoPage } from '../bindings/PhotoPage';
import type { PhotoSummary } from '../bindings/PhotoSummary';
import type { AboutInfo } from '../bindings/AboutInfo';
import type { UpdateInfo } from '../bindings/UpdateInfo';
import type { Settings } from '../bindings/Settings';

export type {
  AboutInfo, AddCheck, AddResult, DiagnosticsBundle, FeedEntry, InviteOut, SharedAlbumInfo, SharedItemInfo, SharedItemsPage, SharedMemberInfo,
  AlbumInfo, AppEvent, BackupPolicy, BackupReport, BackupStatus, AppState, CreatedLibrary, DeviceInfo, DeviceStorageInfo, ExportReport, FreeUpReport, Health, ImportSummary,
  LibraryCounts, PageCursor, PairStart, PhotoDetail, PhotoPage, PhotoSummary, Settings,
};

export type BlobKind = 'thumb' | 'preview' | 'original' | 'poster';

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
export function blobUrl(id: string, kind: BlobKind, album?: string): string {
  // A photo of a shared album is addressed through the album: `kind=album:<album id>:<kind>`.
  if (album) kind = `album:${album}:${kind}` as BlobKind;
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
  /** Let in (or refuse) the device rejoining through this one with the recovery phrase. */
  about: () => call<AboutInfo>('about'),
  checkForUpdate: () => call<UpdateInfo | null>('check_for_update'),
  dismissUpdate: () => call<{}>('dismiss_update'),
  setUpdateChecks: (on: boolean) => call<{}>('set_update_checks', { on }),
  thirdPartyLicences: () => call<string>('third_party_licences'),
  answerRecovery: (accept: boolean) => call<{}>('answer_recovery', { accept }),
  /** Make a new recovery phrase; `phrase` is the current one. The result is the only copy. */
  replaceRecoveryPhrase: (phrase: string) => call<CreatedLibrary>('replace_recovery_phrase', { phrase }),
  dismissAlert: (device_id: string) => call<{}>('dismiss_alert', { device_id }),
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
  /** Phone: the person chooses photos (`files`: from anywhere, else the gallery) and they are imported. */
  importPicked: (files: boolean) => call<ImportSummary>('import_picked', { files }),
  pinOriginal: (id: string, pin: boolean) => call<{}>('pin_original', { id, pin }),
  fetchNow: (id: string, kind: BlobKind) => call<{}>('fetch_now', { id, kind }),
  albums: () => call<AlbumInfo[]>('albums'),
  createAlbum: (name: string) => call<AlbumInfo>('create_album', { name }),
  albumAdd: (album: string, ids: string[]) => call<{}>('album_add', { album, ids }),
  albumRemove: (album: string, ids: string[]) => call<{}>('album_remove', { album, ids }),
  albumPage: (album: string, cursor: PageCursor | null, limit = 500) => call<PhotoPage>('album_page', { album, cursor, limit }),
  devices: () => call<DeviceInfo[]>('devices'),
  deviceStorage: () => call<DeviceStorageInfo[]>('device_storage'),
  /** `phrase`: the recovery phrase, needed to remove an admin device or when this device is no admin. */
  revokeDevice: (id: string, phrase?: string) => call<{}>('revoke_device', { id, phrase: phrase ?? null }),
  /** `phrase`: the recovery phrase, needed when this device is no admin. */
  /** Make a device an admin; needs the recovery phrase. */
  makeAdmin: (id: string, phrase: string) => call<{}>('make_admin', { id, phrase }),
  setDeviceRole: (id: string, role: string, phrase?: string) => call<{}>('set_device_role', { id, role, phrase: phrase ?? null }),
  health: () => call<Health>('health'),
  freeUpSpace: (dry_run: boolean) => call<FreeUpReport>('free_up_space', { dry_run }),
  settings: () => call<Settings>('settings'),
  setRedundancyTarget: (copies: number) => call<{}>('set_redundancy_target', { copies }),
  setRelay: (relay: string) => call<{}>('set_relay', { relay }),
  renameDevice: (name: string) => call<{}>('rename_device', { name }),
  renameAlbum: (album: string, name: string) => call<{}>('rename_album', { album, name }),
  setPoster: (id: string, data: string) => call<{}>('set_poster', { id, data }),
  backupStatus: () => call<BackupStatus>('backup_status'),
  backupConfigure: (policy: BackupPolicy) => call<BackupStatus>('backup_configure', { policy }),
  backupRun: () => call<BackupReport>('backup_run'),
  sharedAlbums: () => call<SharedAlbumInfo[]>('shared_albums'),
  sharedCreate: (name: string) => call<SharedAlbumInfo>('shared_create', { name }),
  sharedInvite: (album: string, role: string, days = 7) => call<InviteOut>('shared_invite', { album, role, days }),
  sharedJoin: (text: string) => call<SharedAlbumInfo>('shared_join', { text }),
  sharedLeave: (album: string) => call<{}>('shared_leave', { album }),
  sharedItems: (album: string, after: string | null = null, limit = 500) => call<SharedItemsPage>('shared_items', { album, after, limit }),
  sharedMembers: (album: string) => call<SharedMemberInfo[]>('shared_members', { album }),
  sharedFeed: (album: string) => call<FeedEntry[]>('shared_feed', { album }),
  sharedAddCheck: (album: string, ids: string[]) => call<AddCheck>('shared_add_check', { album, ids }),
  sharedAdd: (album: string, ids: string[], location: string | null = null, remember = false) => call<AddResult>('shared_add', { album, ids, location, remember }),
  sharedSave: (album: string, ids: string[]) => call<number>('shared_save', { album, ids }),
  sharedSettings: (album: string, keep_originals: boolean | null = null, location: string | null = null) => call<{}>('shared_settings', { album, keep_originals, location }),
  sharedSeen: (album: string) => call<{}>('shared_seen', { album }),
  sharedFetch: (album: string, id: string, kind: BlobKind) => call<{}>('shared_fetch', { album, id, kind }),
  sharedRemoveMember: (album: string, member: string) => call<{}>('shared_remove_member', { album, member }),
  syncBurst: (seconds = 10) => call<import('../bindings/BurstReport').BurstReport>('sync_burst', { seconds }),
  deviceLink: () => call<string>('device_link'),
  diagnostics: () => call<DiagnosticsBundle>('diagnostics'),
  exportLibrary: (dest: string) => call<ExportReport>('export_library', { dest }),
  watchFolder: (dir: string) => call<{}>('watch_folder', { dir }),
};
