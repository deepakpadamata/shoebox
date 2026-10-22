// App-wide reactive state: what the shell needs everywhere. Screens keep their own data and
// refetch when `app.version` changes (the app core says something changed).
import { api, onAppEvent, type AppEvent, type AppState, type LibraryCounts } from './api';

export const app = $state({
  loading: true,
  error: '',
  state: null as AppState | null,
  counts: null as LibraryCounts | null,
  /** Bumped whenever the library or the device list changed. */
  version: 0,
  devicesVersion: 0,
  /** Peers currently connected, by device id (for the "n devices online" line). */
  pairing: null as null | { sas: string; peer_name: string; peer_role: string },
  pairingError: '',
  importProgress: null as null | { done: number; total: number },
  removed: false,
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

export async function refreshState() {
  app.state = await api.appState();
  if (app.state.has_library) app.counts = await api.counts();
}

let countsTimer: ReturnType<typeof setTimeout> | undefined;

function handle(e: AppEvent) {
  if (e === 'LibraryChanged') {
    app.version++;
    clearTimeout(countsTimer);
    countsTimer = setTimeout(() => api.counts().then((c) => (app.counts = c)).catch(() => {}), 400);
  } else if (e === 'DevicesChanged' || e === 'LibraryRotated') {
    app.devicesVersion++;
    app.version++;
  } else if (e === 'RemovedFromLibrary') {
    app.removed = true;
  } else if (e === 'PairingDone') {
    app.pairing = null;
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
    } else if ('PeerConnected' in e || 'PeerDisconnected' in e) {
      app.devicesVersion++;
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
    await refreshState();
  } catch (e) {
    app.error = e instanceof Error ? e.message : String(e);
  } finally {
    app.loading = false;
  }
  onAppEvent(handle);
}
