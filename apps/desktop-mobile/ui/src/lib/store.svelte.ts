// App-wide reactive state: what the shell needs everywhere. Screens keep their own data and
// refetch when `app.version` changes (the app core says something changed).
import { mergeAlbums, type AlbumEntry } from './albumlist';
import { api, loadNativeVideo, onAppEvent, type AppEvent, type AppState, type LibraryCounts, type SafetyStatus } from './api';

/** Small UI signals between screens: the phone's Albums header asks the Albums screen to open its ＋ menu. */
export const ui = $state({ albumsMenu: 0 });

export const app = $state({
  loading: true,
  /** First-run set-up is not over: the library exists, but the last steps (recovery kit, bringing photos in) are still showing. */
  setup: false,
  error: '',
  state: null as AppState | null,
  /** The one answer to "are my photos safe?" (U1-07), computed in Rust. */
  backup_status: null as SafetyStatus | null,
  counts: null as LibraryCounts | null,
  /** Personal and shared albums in one list (U1-19); the sidebar and the Albums screen read it. */
  albums: [] as AlbumEntry[],
  /** Bumped whenever photos or the device list changed. */
  version: 0,
  /** Bumped when an album changed (name, cover, photos). */
  albumsVersion: 0,
  /** Bumped, at most every few seconds, when which devices keep which photos changed. */
  holdingsVersion: 0,
  devicesVersion: 0,
  /** Bumped when something changed in a shared album. */
  sharedVersion: 0,
  /** Bumped when a screen asks the shell to open the import sheet. */
  importRequest: 0,
  /** Peers currently connected, by device id (for the "n devices online" line). */
  pairing: null as null | { sas: string; peer_name: string; peer_role: string },
  pairingError: '',
  /** Bumped when a pairing this device took part in finished. */
  pairVersion: 0,
  /** The Devices screen's "Add a device" sheet is open (it shows the pairing code itself). */
  addDeviceOpen: false,
  importProgress: null as null | { done: number; total: number },
  /** The Google Photos import: running, unfinished, or just done (with its report). */
  archive: null as null | import('../bindings/ArchiveImportInfo').ArchiveImportInfo,
  removed: false,
  /** Other devices reachable right now, and recent at-risk counts for the ETA. */
  peersOnline: 0,
  riskSamples: [] as { t: number; n: number }[],
  backup: null as null | import('./api').BackupStatus,
  backupProgress: null as null | { done: number; total: number },
  /** What the sync manager is doing right now: peers mid-sync, and the last failure. */
  syncing: [] as string[],
  syncError: '',
  online: 0,
  toast: '',
});

let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function toast(message: string) {
  app.toast = message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (app.toast = ''), 4000);
}

export function fail(e: unknown) {
  toast(e instanceof Error ? e.message : String(e));
}

/** Remember how fast the at-risk count falls, to estimate when backup will be done. */
export function noteRisk(n: number) {
  const t = Date.now();
  app.riskSamples = [...app.riskSamples.filter((s) => t - s.t < 120_000), { t, n }];
}

/** Seconds until nothing is at risk at the recent pace, or null when it is not falling. */
export function etaSeconds(): number | null {
  const s = app.riskSamples;
  if (s.length < 2) return null;
  const [first, last] = [s[0], s[s.length - 1]];
  const dt = (last.t - first.t) / 1000;
  const rate = (first.n - last.n) / dt;
  return rate > 0 && dt >= 5 ? Math.ceil(last.n / rate) : null;
}

export async function refreshPeers() {
  const list = await api.devices();
  app.peersOnline = list.filter((d) => !d.is_this_device && d.online && !d.revoked).length;
}

export async function refreshAlbums() {
  const [personal, shared] = await Promise.all([api.albums(), api.sharedAlbums().catch(() => [])]);
  app.albums = mergeAlbums(personal, shared);
}

export async function refreshState() {
  app.state = await api.appState();
  if (app.state.has_library) {
    app.counts = await api.counts();
    noteRisk(app.counts.at_risk);
    refreshPeers().catch(() => {});
    app.backup = await api.backupStatus().catch(() => null);
    app.backup_status = await api.safetyStatus().catch(() => null);
    refreshAlbums().catch(() => {});
    app.archive = await api.archiveImportStatus().catch(() => null);
  }
}

/** The window is showing. While it is hidden (minimised, in the tray, the phone app in the
 *  background) nothing is reloaded: what changed is noted and reloaded once when it shows again. */
export const page = $state({ visible: typeof document === 'undefined' || !document.hidden });

type Versioned = 'version' | 'albumsVersion' | 'holdingsVersion' | 'devicesVersion' | 'sharedVersion';
const missed = new Set<Versioned | 'counts' | 'albums' | 'state'>();

/** Tell the screens that `what` changed, now or when the window shows again. */
function bump(what: Versioned) {
  if (page.visible) app[what]++;
  else missed.add(what);
}

let countsTimer: ReturnType<typeof setTimeout> | undefined;
let albumsTimer: ReturnType<typeof setTimeout> | undefined;
function refreshCounts() {
  if (!page.visible) return void missed.add('counts');
  clearTimeout(countsTimer);
  countsTimer = setTimeout(() => api.counts().then((c) => { app.counts = c; noteRisk(c.at_risk); }).catch(() => {}), 400);
}
function refreshAlbumsSoon() {
  if (!page.visible) return void missed.add('albums');
  clearTimeout(albumsTimer);
  albumsTimer = setTimeout(() => refreshAlbums().catch(() => {}), 400);
}
function refreshStateSoon() {
  if (!page.visible) return void missed.add('state');
  refreshState().catch(() => {});
}

// Holdings move with every blob a fetch or a backup brings in: screens hear of it at most this often.
const HOLDINGS_EVERY_MS = 5000;
let holdingsAt = 0;
let holdingsTimer: ReturnType<typeof setTimeout> | undefined;
function holdingsChanged() {
  refreshCounts();
  if (holdingsTimer) return;
  const wait = Math.max(0, holdingsAt + HOLDINGS_EVERY_MS - Date.now());
  holdingsTimer = setTimeout(() => {
    holdingsTimer = undefined;
    holdingsAt = Date.now();
    bump('holdingsVersion');
  }, wait);
}

if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    page.visible = !document.hidden;
    if (!page.visible || missed.size === 0) return;
    const was = new Set(missed);
    missed.clear();
    if (was.has('state')) refreshState().catch(() => {});
    else {
      if (was.has('counts')) refreshCounts();
      if (was.has('albums')) refreshAlbumsSoon();
    }
    for (const w of ['version', 'albumsVersion', 'holdingsVersion', 'devicesVersion', 'sharedVersion'] as const) if (was.has(w)) app[w]++;
  });
}

function handle(e: AppEvent) {
  if (e === 'LibraryChanged') {
    bump('version');
    refreshCounts();
    // An album's count and cover follow its photos.
    refreshAlbumsSoon();
  } else if (e === 'AlbumsChanged') {
    bump('albumsVersion');
    refreshAlbumsSoon();
  } else if (e === 'HoldingsChanged') {
    holdingsChanged();
  } else if (e === 'DevicesChanged' || e === 'LibraryRotated') {
    bump('devicesVersion');
    bump('version');
    // This device's own name or role may be what changed (it refreshes the peer count too).
    refreshStateSoon();
  } else if (e === 'ArchiveImport') {
    api.archiveImportStatus().then((a) => (app.archive = a)).catch(() => {});
  } else if (e === 'SecurityChanged') {
    refreshStateSoon();
  } else if (e === 'RemovedFromLibrary') {
    app.removed = true;
  } else if (e === 'PairingDone') {
    app.pairing = null;
    app.pairVersion++;
    refreshState().catch(() => {});
  } else if (typeof e === 'object') {
    if ('PairingCode' in e) app.pairing = e.PairingCode;
    else if ('PairingFailed' in e) {
      app.pairing = null;
      app.pairingError = e.PairingFailed.reason;
    } else if ('ImportProgress' in e) {
      const { done, total } = e.ImportProgress;
      // The photos it brings in send their own LibraryChanged.
      app.importProgress = done >= total ? null : { done, total };
    } else if ('Safety' in e) {
      app.backup_status = e.Safety.status;
    } else if ('SharedChanged' in e) {
      bump('sharedVersion');
      refreshAlbumsSoon();
    } else if ('PeerConnected' in e || 'PeerDisconnected' in e) {
      bump('devicesVersion');
    } else if ('BackupProgress' in e) {
      const { done, total } = e.BackupProgress;
      app.backupProgress = done >= total ? null : { done, total };
    } else if ('GalleryDeletionsHeld' in e) {
      if (app.backup) app.backup = { ...app.backup, gallery_deletions_held: e.GalleryDeletionsHeld.count };
    } else if ('BackupPaused' in e) {
      if (app.backup) app.backup = { ...app.backup, paused: e.BackupPaused.reason };
    } else if ('Sync' in e) {
      const { peer, phase } = e.Sync;
      if (phase === 'Started') {
        app.syncing = [...new Set([...app.syncing, peer])];
      } else {
        app.syncing = app.syncing.filter((p) => p !== peer);
        app.syncError = phase.startsWith('Failed') ? phase : phase === 'Finished' ? '' : app.syncError;
      }
    }
  }
}

export async function start() {
  try {
    await Promise.all([refreshState(), loadNativeVideo()]);
  } catch (e) {
    app.error = e instanceof Error ? e.message : String(e);
  } finally {
    app.loading = false;
  }
  onAppEvent(handle);
}
