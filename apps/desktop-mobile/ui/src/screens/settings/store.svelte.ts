// What the settings groups share (U1-14): the settings record and how to save a change.
import { api, type Settings } from '../../lib/api';
import { fail, toast } from '../../lib/store.svelte';

export const cfg = $state({ s: null as Settings | null });

export async function loadSettings() {
  try {
    cfg.s = await api.settings();
  } catch (e) {
    fail(e);
  }
}

/** Run a change, say it was saved, and read the settings again. */
export async function save<T>(f: () => Promise<T>, msg: string) {
  try {
    await f();
    toast(msg);
    await loadSettings();
  } catch (e) {
    fail(e);
  }
}
