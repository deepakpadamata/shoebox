// What the Devices panels show that moves on its own: transfers, who keeps how much, and search
// indexing. One poller serves every panel on screen (the phone's Backup page shows two), runs only
// while at least one is open, and stops while the window is hidden.
import { GrowthRates } from './activity';
import { api, type DeviceStorageInfo, type SearchStatus, type Transfers } from './api';
import { page } from './store.svelte';

export const live = $state({
  transfers: null as Transfers | null,
  storage: [] as DeviceStorageInfo[],
  /** Photos in the library, the scale the storage bars share. */
  total: 0,
  rates: {} as Record<string, number>,
  search: null as SearchStatus | null,
});

const TRANSFERS_MS = 1500;
const STORAGE_MS = 10_500;
const SEARCH_MS = 10_000;

const growth = new GrowthRates();
let watchers = 0;
let searchWatchers = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let storageAt = 0;
let searchAt = 0;

export async function loadStorage() {
  const [s, c] = await Promise.all([api.deviceStorage(), api.counts()]);
  live.storage = s;
  live.total = c.photos;
  growth.add(s);
  live.rates = growth.rates();
  storageAt = Date.now();
}

function loadSearch() {
  searchAt = Date.now();
  api.searchStatus().then((st) => (live.search = st)).catch(() => {});
}

function tick() {
  timer = undefined;
  if (watchers === 0 || !page.visible) return;
  api.transfers().then((t) => (live.transfers = t)).catch(() => {});
  const now = Date.now();
  if (now - storageAt >= STORAGE_MS) loadStorage().catch(() => {});
  if (searchWatchers > 0 && now - searchAt >= SEARCH_MS) loadSearch();
  timer = setTimeout(tick, TRANSFERS_MS);
}

function restart() {
  clearTimeout(timer);
  timer = undefined;
  tick();
}

$effect.root(() => {
  $effect(() => {
    // Back from hidden: catch up at once rather than on the next tick.
    if (page.visible && watchers > 0 && timer === undefined) restart();
  });
});

/** Keep the numbers current while a panel is open; `search` also polls indexing. Returns the stop. */
export function watch({ search = false }: { search?: boolean } = {}): () => void {
  watchers++;
  if (search) {
    searchWatchers++;
    searchAt = 0;
  }
  // A panel that opens shows fresh numbers, not ones from when the last one closed.
  storageAt = 0;
  restart();
  return () => {
    watchers--;
    if (search) searchWatchers--;
    if (watchers === 0) {
      clearTimeout(timer);
      timer = undefined;
    }
  };
}
