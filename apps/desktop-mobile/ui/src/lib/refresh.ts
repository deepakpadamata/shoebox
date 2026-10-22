// What "pull to refresh" does: look at the camera roll, sync with the other devices for a few
// seconds, then reload what is on screen.
import { api } from './api';
import { app, refreshState, toast } from './store.svelte';

export async function refreshEverything(): Promise<void> {
  try {
    if (app.backup?.policy.enabled) await api.backupRun().catch(() => {});
    await api.syncBurst(8).catch(() => {});
    await refreshState();
    app.version++;
    app.sharedVersion++;
    app.devicesVersion++;
    toast('Up to date');
  } catch {
    toast('Could not refresh. Check that your other devices are reachable.');
  }
}
