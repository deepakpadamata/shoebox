// App-wide reactive state: what the shell needs everywhere. Screens keep their own data and
// refetch when `app.version` changes (the app core says something changed).
import { api, loadMediaBase, onAppEvent, type AppEvent, type AppState, type LibraryCounts } from './api';

export const app = $state({
  loading: true,
  error: '',
  state: null as AppState | null,
  counts: null as LibraryCounts | null,
  /** Bumped whenever the library or the device list changed. */
  version: 0,
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
  importProgress: null as null | { done: number; total: number },
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

export async function refreshState() {
  app.state = await api.appState();
  if (app.state.has_library) {
    app.counts = await api.counts();
    noteRisk(app.counts.at_risk);
    refreshPeers().catch(() => {});
    app.backup = await api.backupStatus().catch(() => null);
  }
}

let countsTimer: ReturnType<typeof setTimeout> | undefined;

function handle(e: AppEvent) {
  if (e === 'LibraryChanged') {
    app.version++;
    clearTimeout(countsTimer);
    countsTimer = setTimeout(() => api.counts().then((c) => { app.counts = c; noteRisk(c.at_risk); }).catch(() => {}), 400);
  } else if (e === 'DevicesChanged' || e === 'LibraryRotated') {
    app.devicesVersion++;
    app.version++;
    refreshPeers().catch(() => {});
    // This device's own name or role may be what changed.
    refreshState().catch(() => {});
  } else if (e === 'SecurityChanged') {
    refreshState().catch(() => {});
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
      app.importProgress = done >= total ? null : { done, total };
      app.version++;
    } else if ('SharedChanged' in e) {
      app.sharedVersion++;
    } else if ('PeerConnected' in e || 'PeerDisconnected' in e) {
      app.devicesVersion++;
    } else if ('BackupProgress' in e) {
      const { done, total } = e.BackupProgress;
      app.backupProgress = done >= total ? null : { done, total };
      app.version++;
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
    await Promise.all([refreshState(), loadMediaBase()]);
  } catch (e) {
    app.error = e instanceof Error ? e.message : String(e);
  } finally {
    app.loading = false;
  }
  onAppEvent(handle);
}
