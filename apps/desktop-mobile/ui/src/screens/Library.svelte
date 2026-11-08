<script lang="ts">
  import Icon from '../components/Icon.svelte';
  // The timeline (M3-06): photos grouped by day, virtualised so 100k photos scroll smoothly,
  // with multi-select and the actions of mockup 2b.
  import { untrack } from 'svelte';
  import { api, type PageCursor, type PhotoPage, type PhotoSummary, type SearchPage } from '../lib/api';
  import { tzOffset } from '../lib/search';
  import { dayLabel, monthLabel, plural } from '../lib/format';
  import { buildLayout, packedLabel, visibleRows } from '../lib/timeline';
  import { longPressDrag, pinch, pullRefresh, stepDensity } from '../lib/gestures';
  import { refreshEverything } from '../lib/refresh';
  import { viewport as media } from '../lib/viewport.svelte';
  import { app, fail, refreshAlbums, toast } from '../lib/store.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import { go } from '../lib/route.svelte';
  import Tile from '../components/Tile.svelte';
  import DateRail from '../components/DateRail.svelte';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';
  import ShareSheet from '../components/ShareSheet.svelte';
  import Viewer from './Viewer.svelte';

  interface Props {
    source: 'all' | 'favorites' | 'album' | 'search' | 'similar' | 'person';
    album?: string;
    /** For `similar`: the photo the others should look like. For `person`: who. */
    photo?: string;
    /** For `search`: the query, and where to report what it was understood as. */
    query?: string;
    onsearch?: (page: Omit<SearchPage, 'items' | 'next'>) => void;
  }
  let { source, album, photo, query = '', onsearch }: Props = $props();
  // Search results ranked by relevance have no day headers.
  let ranked = $state(false);

  const GAP = 2;
  const HEAD = 44;
  // Replaced whole, never changed in place: no need for deep reactivity on thousands of photos.
  let items = $state.raw<PhotoSummary[]>([]);
  let loaded = $state(false);
  // Every photo is in `items` (not only the newest, still arriving).
  let complete = $state(false);
  let width = $state(900);
  let viewport = $state(600);
  let scrollTop = $state(0);
  let scroller: HTMLDivElement | undefined = $state();
  let selected = $state<string[]>([]);
  let lastClicked = $state(-1);
  let viewing = $state<number | null>(null);
  // Close the viewer, wait for it to give back its history entry, then show the similar photos.
  function showSimilar(id: string) {
    viewing = null;
    let done = false;
    const next = () => {
      if (done) return;
      done = true;
      go(`similar/${id}`);
    };
    addEventListener('popstate', next, { once: true });
    setTimeout(next, 300);
  }
  let albums = $state<{ id: string; name: string }[]>([]);
  let albumSheet = $state(false);
  let shareSheet = $state(false);
  let shareButton = $state<HTMLButtonElement>();
  let newAlbum = $state('');
  const selecting = $derived(selected.length > 0);
  const selectedSet = $derived(new Set(selected));
  const target = $derived(app.counts?.redundancy_target ?? 2);

  const DENSITIES = [72, 96, 130, 180];
  let density = $state(Number(localStorage.getItem('density')) || 0);
  const minTile = $derived(density || (media.mobile ? 96 : 140));
  // Pinch to change the grid density; the choice is remembered.
  // Past the smallest tiles the timeline groups by month (U1-18); pinching in leaves it again.
  let monthView = $state(localStorage.getItem('monthView') === '1');
  function setDensity(scale: number) {
    if (monthView) {
      if (scale > 1) setMonthView(false);
      return;
    }
    const next = stepDensity(minTile, scale, DENSITIES);
    if (scale < 1 && next === minTile && minTile <= DENSITIES[0]) { setMonthView(true); return; }
    density = next;
    localStorage.setItem('density', String(density));
  }
  function setMonthView(on: boolean) {
    monthView = on;
    try { localStorage.setItem('monthView', on ? '1' : '0'); } catch { /* storage may be blocked */ }
  }
  // Long-press a tile, then drag across others to select them (mockup 2b).
  function indexOf(el: Element | null): number {
    const t = el?.closest('[data-index]');
    return t ? Number(t.getAttribute('data-index')) : -1;
  }
  // The drag selects the run from the pressed photo to the one under the finger, on top of what
  // was selected before: nothing is skipped when the view scrolls fast, and going back unselects.
  let dragFrom = -1;
  let dragTo = -1;
  let dragBase: string[] = [];
  function dragStart(el: Element | null) {
    dragFrom = dragTo = -1;
    dragBase = selected;
    dragOver(el);
  }
  function dragOver(el: Element | null) {
    const i = indexOf(el);
    if (i < 0 || !items[i] || i === dragTo) return;
    if (dragFrom < 0) dragFrom = i;
    dragTo = i;
    const run = items.slice(Math.min(dragFrom, i), Math.max(dragFrom, i) + 1).map((p) => p.id);
    selected = [...new Set([...dragBase, ...run])];
    lastClicked = i;
  }
  // First sync: a new device has the list of photos long before it has their previews.
  let samples = $state<{ t: number; n: number }[]>([]);
  const wantThumbs = $derived(items.filter((p) => !p.is_video));
  const missingThumbs = $derived(wantThumbs.filter((p) => !p.has_thumb).length);
  $effect(() => {
    const n = missingThumbs;
    const t = Date.now();
    untrack(() => { samples = [...samples.filter((s) => t - s.t < 120_000), { t, n }]; });
  });
  const gettingEta = $derived.by(() => {
    if (samples.length < 2) return '';
    const [a, b] = [samples[0], samples[samples.length - 1]];
    const dt = (b.t - a.t) / 1000;
    const rate = (a.n - b.n) / dt;
    if (!(rate > 0) || dt < 5) return '';
    const s = Math.ceil(b.n / rate);
    return s < 90 ? ' about a minute left' : s < 3600 ? ` about ${Math.ceil(s / 60)} min left` : ` about ${Math.ceil(s / 3600)} h left`;
  });
  const firstSync = $derived(source === 'all' && wantThumbs.length > 0 && missingThumbs > 0 && app.peersOnline > 0);

  const layout = $derived(buildLayout(items, { width, minTile: monthView ? DENSITIES[0] : minTile, gap: GAP, headHeight: HEAD, days: !ranked, pack: !monthView, group: monthView ? 'month' : 'day' }));
  const range = $derived(visibleRows(layout, scrollTop, viewport));
  const shown = $derived(layout.rows.slice(range[0], range[1]));
  // The date scroller, once there is more than a couple of screens to scroll through.
  const showRail = $derived(loaded && items.length > 0 && layout.height > viewport * 2);

  // The first time, the newest photos show as soon as a screenful has arrived and the rest
  // follows; later reloads keep the grid as it is until the new list is whole.
  const FIRST_PAGE = 120;
  const PAGE = 2000;
  // One load at a time: a change during a load asks for one more pass after it.
  let loading: Promise<void> | null = null;
  let again = false;
  // Bumped when the source changes: a load for the old one stops and the next pass starts over.
  let generation = 0;

  function reload() {
    if (loading) {
      again = true;
      return;
    }
    loading = (async () => {
      try {
        do {
          again = false;
          await load();
        } while (again);
      } catch (e) {
        fail(e);
      } finally {
        loading = null;
      }
    })();
  }

  async function load() {
    const gen = generation;
    const keep = (all: PhotoSummary[]) => (source === 'favorites' ? all.filter((p) => p.favorite) : all.slice());
    const progressive = !loaded;
    const out: PhotoSummary[] = [];
    let cursor: PageCursor | null = null;
    if (source === 'person') {
      items = await api.personPhotos(photo!);
      if (gen !== generation) return;
      loaded = true;
      complete = true;
      return;
    }
    if (source === 'search' || source === 'similar') {
      let at: number | null = null;
      for (let size = progressive ? FIRST_PAGE : PAGE; ; size = PAGE) {
        const page: SearchPage =
          source === 'similar' ? await api.similar(photo!, at, size) : await api.search(query, at, size, tzOffset());
        if (gen !== generation) return;
        out.push(...page.items);
        if (at === null) {
          ranked = page.ranked;
          onsearch?.({ total: page.total, ranked: page.ranked, filters: page.filters, free_text: page.free_text });
        }
        if (page.next === null) break;
        at = page.next;
        if (progressive) {
          items = out.slice();
          loaded = true;
        }
      }
      items = out.slice();
      loaded = true;
      complete = true;
      const ids = new Set(items.map((p) => p.id));
      selected = selected.filter((id) => ids.has(id));
      return;
    }
    for (let size = progressive ? FIRST_PAGE : PAGE; ; size = PAGE) {
      const page: PhotoPage = source === 'album' && album ? await api.albumPage(album, cursor, size) : await api.libraryPage(cursor, size);
      if (gen !== generation) return;
      out.push(...page.items);
      if (!page.next) break;
      cursor = page.next;
      if (progressive) {
        items = keep(out);
        loaded = true;
      }
    }
    items = keep(out);
    loaded = true;
    complete = true;
    const ids = new Set(items.map((p) => p.id));
    selected = selected.filter((id) => ids.has(id));
  }

  // Reload when the library changes (debounced); start over when the source changes.
  let timer: ReturnType<typeof setTimeout> | undefined;
  let showing = '';
  $effect(() => {
    app.version;
    const key = `${source}/${album ?? ''}/${query}`;
    untrack(() => {
      clearTimeout(timer);
      if (key !== showing) {
        showing = key;
        generation++;
        loaded = false;
        complete = false;
        reload();
      } else {
        timer = setTimeout(reload, 350);
      }
    });
  });

  async function selectAll() {
    while (!complete && loading) await loading;
    selected = items.map((p) => p.id);
  }

  function click(i: number, e: MouseEvent) {
    const id = items[i].id;
    // The circle on a tile selects it, also to start selecting.
    const onCircle = (e.target as Element | null)?.closest('.sel') !== null;
    if (e.shiftKey && lastClicked >= 0) {
      const [a, b] = [Math.min(lastClicked, i), Math.max(lastClicked, i)];
      selected = [...new Set([...selected, ...items.slice(a, b + 1).map((p) => p.id)])];
    } else if (selecting || onCircle || e.metaKey || e.ctrlKey) {
      selected = selectedSet.has(id) ? selected.filter((x) => x !== id) : [...selected, id];
      lastClicked = i;
    } else {
      viewing = i;
    }
  }

  // The circle by a date selects the whole day, or clears it when all of it is selected.
  function dayIds(from: number, count: number) {
    return items.slice(from, from + count).map((p) => p.id);
  }
  function toggleDay(from: number, count: number) {
    const ids = dayIds(from, count);
    if (ids.every((id) => selectedSet.has(id))) {
      const drop = new Set(ids);
      selected = selected.filter((id) => !drop.has(id));
    } else {
      selected = [...new Set([...selected, ...ids])];
    }
    lastClicked = from + count - 1;
  }

  async function act<T>(f: () => Promise<T>, done?: string) {
    try {
      await f();
      if (done) toast(done);
    } catch (e) {
      fail(e);
    }
  }

  const allFavorite = $derived(selected.length > 0 && selected.every((id) => items.find((p) => p.id === id)?.favorite));

  function keydown(e: KeyboardEvent) {
    if (viewing !== null) return;
    if (e.key === 'Escape') selected = [];
    if ((e.key === 'Delete' || e.key === 'Backspace') && selecting && !(e.target instanceof HTMLInputElement)) askDelete();
    if (e.key === 's' && selecting && !(e.target instanceof HTMLInputElement) && !(e.metaKey || e.ctrlKey)) shareSheet = true;
    if ((e.metaKey || e.ctrlKey) && e.key === 'a' && !(e.target instanceof HTMLInputElement)) {
      e.preventDefault();
      selectAll();
    }
  }

  // ----- selection actions (U1-17) -----
  let moreOpen = $state(false);
  let confirmOpen = $state(false);
  let ctx = $state<{ x: number; y: number } | null>(null);
  const deviceWords = $derived((app.backup_status?.devices ?? 0) > 1 ? `all ${app.backup_status!.devices} devices` : 'this device');
  function askDelete() { if (selected.length) confirmOpen = true; }
  // Right-click on a computer: the menu acts on the selection, which first takes in the photo
  // clicked when it is not part of it.
  function context(i: number, e: MouseEvent) {
    if (media.mobile) return;
    e.preventDefault();
    const id = items[i].id;
    if (!selectedSet.has(id)) selected = [id];
    lastClicked = i;
    ctx = { x: e.clientX, y: e.clientY };
  }

  async function removeSelected() {
    const ids = [...selected];
    await act(() => api.delete(ids), `${plural(ids.length, 'photo')} moved to Recently Deleted`);
    selected = [];
  }

  async function openAlbumSheet() {
    albums = await api.albums();
    albumSheet = true;
  }

  async function addToAlbum(id: string) {
    const ids = [...selected];
    await act(() => api.albumAdd(id, ids), `Added ${plural(ids.length, 'photo')}`);
    albumSheet = false;
    selected = [];
  }

  async function createAndAdd() {
    if (!newAlbum.trim()) return;
    try {
      const a = await api.createAlbum(newAlbum);
      newAlbum = '';
      await addToAlbum(a.id);
    } catch (e) {
      fail(e);
    }
  }

  function jump(delta: number) {
    if (viewing === null) return;
    viewing = Math.min(items.length - 1, Math.max(0, viewing + delta));
  }
</script>

<svelte:window onkeydown={keydown} />


<div class="wrap">
  {#if firstSync}
    <div class="getting" role="status">
      <b>Getting your photos</b>
      <span class="muted small">{(wantThumbs.length - missingThumbs).toLocaleString('en')} of {wantThumbs.length.toLocaleString('en')} previews are on this device.{gettingEta}</span>
      <ProgressBar total={wantThumbs.length} segments={[{ value: wantThumbs.length - missingThumbs, tone: 'info' }]} label="Previews downloaded" />
    </div>
  {/if}
  <div class="body">
  <div class="scroller" id="timeline" class:railed={showRail} bind:this={scroller} bind:clientWidth={width} bind:clientHeight={viewport}
    onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
    use:pinch={{ onEnd: setDensity }}
    use:pullRefresh={{ onRefresh: refreshEverything }}
    use:longPressDrag={{ onStart: dragStart, onOver: dragOver, onEnd: () => (dragFrom = dragTo = -1) }}>
    {#if !loaded || (items.length === 0 && !complete)}
      <div class="empty">Loading…</div>
    {:else if items.length === 0 && source === 'search'}
      <div class="empty">
        <b>{query.trim() ? 'No photos match' : 'Search your library'}</b>
        <ul class="tips muted small">
          {#if query.trim()}<li>Remove a chip to widen the search</li>{/if}
          <li>Try a year, a month, a place or a camera: "2019", "March", "Lisbon"</li>
          <li>"videos", "screenshots", "selfies" and "favourites" work too</li>
          <li>So do "last summer", "weekends" and "Christmas"</li>
        </ul>
      </div>
    {:else if items.length === 0 && source === 'all' && (app.importProgress || app.backupProgress)}
      <!-- The first import is running: say how far it is rather than "No photos yet". -->
      {@const p = app.importProgress ?? app.backupProgress!}
      <div class="empty" role="status">
        <b>Bringing your photos in</b>
        <span class="muted">{p.done.toLocaleString('en')} of {p.total.toLocaleString('en')}. They show up here as they arrive.</span>
        <ProgressBar total={p.total} segments={[{ value: p.done, tone: 'info' }]} label="Importing" />
      </div>
    {:else if items.length === 0}
      <div class="empty">
        <b>{source === 'favorites' ? 'No favorites yet' : source === 'album' ? 'This album is empty' : 'No photos yet'}</b>
        <span class="muted">{source === 'all' ? 'Bring photos in from a folder, or add another device to start backing up.' : source === 'favorites' ? 'Tap the heart on a photo to find it here quickly.' : 'Add photos from the album page, or select photos in Photos and choose Add to album.'}</span>
        {#if source === 'all'}
          <div class="row-flex" style="justify-content:center;margin-top:10px">
            {#if media.mobile}
              <Button kind="primary" onclick={() => go(app.backup && app.backup.permission !== 'unavailable' ? 'settings' : 'backup')}>{app.backup && app.backup.permission !== 'unavailable' ? 'Turn on camera backup' : 'Add a device'}</Button>
            {:else}
              <Button kind="primary" onclick={() => app.importRequest++}>Import photos</Button>
              <Button onclick={() => go('backup/devices')}>Add a device</Button>
            {/if}
          </div>
        {:else}
          <div style="margin-top:10px"><Button kind="primary" onclick={() => go('library')}>Open the library</Button></div>
        {/if}
      </div>
    {:else}
      <div class="canvas" style="height:{layout.height}px">
        {#each shown as row (row.top)}
          {#if row.type === 'head'}
            {@const all = dayIds(row.from, row.count).every((id) => selectedSet.has(id))}
            <div class="head" class:selecting style="top:{row.top}px;height:{row.height}px">
              <!-- Circle and date are one target while selecting; otherwise only the circle is. -->
              <button class="day" class:all aria-pressed={all} aria-label="Select {row.month ? monthLabel(row.day) : row.days ? packedLabel(row.days) : dayLabel(row.day)}" title={all ? 'Clear this selection' : row.month ? 'Select this month' : row.days ? 'Select these days' : 'Select this day'}
                onclick={() => toggleDay(row.from, row.count)}>
                <span class="circle"><svg viewBox="0 0 24 24"><path d="M7 12.5l3.2 3.2L17 9" /></svg></span>
                <b>{row.month ? monthLabel(row.day) : row.days ? packedLabel(row.days) : dayLabel(row.day)}</b>
              </button>
              <span class="grow"></span>
              <span class="n">{row.count.toLocaleString('en')}</span>
            </div>
          {:else}
            <div class="tiles" style="top:{row.top}px;height:{row.height}px;gap:{GAP}px">
              {#each items.slice(row.from, row.to) as p, k (p.id)}
                <Tile index={row.from + k} id={p.id} size={layout.tile} favorite={p.favorite} video={p.is_video} live={p.live} copies={p.copies} {target}
                  takenAt={p.taken_at} hasThumb={p.has_thumb} hasPoster={p.has_poster} needsPoster={p.needs_poster} hasOriginal={p.has_original} backedUp={p.backed_up} selected={selectedSet.has(p.id)} {selecting} onclick={(e) => click(row.from + k, e)} oncontextmenu={(e) => context(row.from + k, e)} />
              {/each}
            </div>
          {/if}
        {/each}
      </div>
      {#if !complete}<div class="more muted small" role="status">Loading older photos…</div>{/if}
      {#if selecting}<div class="barroom" aria-hidden="true"></div>{/if}
    {/if}
  </div>
  {#if selecting}
    <!-- Labelled actions, Delete last and red (U1-17). On a phone the bar takes the place of the
         tab bar; on a computer it is a toolbar over the grid. -->
    <div class="selbar" class:phone={media.mobile} role="toolbar" aria-label="{selected.length} selected">
      <button class="clear" title="Clear selection (Esc)" aria-label="Clear selection" onclick={() => (selected = [])}><Icon name="close" size={20} /></button>
      <b class="count">{selected.length.toLocaleString('en')} selected</b>
      <span class="grow"></span>
      <button class="act" bind:this={shareButton} title="Share (S)" onclick={() => (shareSheet = true)}><Icon name="share" size={22} /><span>Share</span></button>
      <button class="act" title="Add to album" onclick={openAlbumSheet}><Icon name="addalbum" size={22} /><span>Add to album</span></button>
      <button class="act" class:on={allFavorite} title={allFavorite ? 'Unfavorite' : 'Favorite'} onclick={() => act(() => api.setFavorite(selected, !allFavorite))}><Icon name="heart" size={22} fill={allFavorite} /><span>{allFavorite ? 'Unfavorite' : 'Favorite'}</span></button>
      <button class="act" title="More" onclick={() => (moreOpen = true)}><Icon name="more" size={22} /><span>More</span></button>
      <button class="act danger" title="Delete from all devices (Delete)" onclick={askDelete}><Icon name="trash" size={22} /><span>Delete</span></button>
    </div>
  {/if}
  {#if showRail}
    <DateRail {layout} {scrollTop} {viewport} onseek={(top) => { if (scroller) scroller.scrollTop = top; }} />
  {/if}
  </div>
</div>

{#if viewing !== null && items[viewing]}
  <Viewer {items} index={viewing} onclose={() => (viewing = null)} onmove={jump} onsimilar={showSimilar} />
{/if}

<ShareSheet open={shareSheet} ids={selected} anchor={() => shareButton?.getBoundingClientRect()} onclose={() => (shareSheet = false)}
  ondone={() => { shareSheet = false; selected = []; }} />

<Sheet open={moreOpen} title="{plural(selected.length, 'photo')} selected" onclose={() => (moreOpen = false)} width={340}>
  <div class="menu">
    {#if app.state?.role !== 'keeper'}<button onclick={() => { moreOpen = false; act(() => Promise.all(selected.map((id) => api.pinOriginal(id, true))), `Kept on ${media.mobile ? 'this phone' : 'this device'}`); }}><Icon name="keep" size={20} /> Keep on {media.mobile ? 'this phone' : 'this device'}</button>{/if}
    {#if source === 'album' && album}
      <button onclick={() => { moreOpen = false; act(() => api.albumRemove(album, selected), 'Removed from the album').then(() => (selected = [])); }}><Icon name="unalbum" size={20} /> Remove from album</button>
      {#if selected.length === 1}<button onclick={() => { moreOpen = false; act(() => api.setAlbumCover(album, selected[0]), 'Cover changed').then(() => { selected = []; refreshAlbums(); }); }}><Icon name="photos" size={20} /> Make cover</button>{/if}
    {/if}
    {#if selected.length === 1}<button onclick={() => { moreOpen = false; showSimilar(selected[0]); }}><Icon name="search" size={20} /> Find similar</button>{/if}
  </div>
</Sheet>

<Sheet open={confirmOpen} title="Delete {plural(selected.length, 'photo')}?" onclose={() => (confirmOpen = false)} width={400}>
  <div class="stack">
    <p>{selected.length === 1 ? 'This photo' : `These ${selected.length.toLocaleString('en')} photos`} will be deleted from {deviceWords}. {selected.length === 1 ? 'It stays' : 'They stay'} in Recently Deleted for 30 days, then {selected.length === 1 ? 'goes' : 'go'} for good.</p>
    <Button kind="danger" block onclick={() => { confirmOpen = false; removeSelected(); }}>Delete {plural(selected.length, 'photo')}</Button>
    <Button block onclick={() => (confirmOpen = false)}>Cancel</Button>
  </div>
</Sheet>

{#if ctx}
  <div class="ctxscrim" role="presentation" onclick={() => (ctx = null)} oncontextmenu={(e) => { e.preventDefault(); ctx = null; }}></div>
  <div class="ctx" role="menu" style="left:{Math.min(ctx.x, innerWidth - 220)}px; top:{Math.min(ctx.y, innerHeight - 300)}px">
    <button role="menuitem" onclick={() => { ctx = null; shareSheet = true; }}>Share…</button>
    <button role="menuitem" onclick={() => { ctx = null; openAlbumSheet(); }}>Add to album…</button>
    <button role="menuitem" onclick={() => { ctx = null; act(() => api.setFavorite(selected, !allFavorite)); }}>{allFavorite ? 'Unfavorite' : 'Favorite'}</button>
    {#if app.state?.role !== 'keeper'}<button role="menuitem" onclick={() => { ctx = null; act(() => Promise.all(selected.map((id) => api.pinOriginal(id, true))), 'Kept on this device'); }}>Keep on this device</button>{/if}
    {#if source === 'album' && album}<button role="menuitem" onclick={() => { ctx = null; act(() => api.albumRemove(album, selected), 'Removed from the album').then(() => (selected = [])); }}>Remove from album</button>{/if}
    {#if selected.length === 1}<button role="menuitem" onclick={() => { const id = selected[0]; ctx = null; showSimilar(id); }}>Find similar</button>{/if}
    <hr />
    <button role="menuitem" class="danger" onclick={() => { ctx = null; askDelete(); }}>Delete…</button>
  </div>
{/if}

<Sheet open={albumSheet} title="Add to album" onclose={() => (albumSheet = false)} width={380}>
  <div class="stack">
    {#each albums as a}
      <Button block onclick={() => addToAlbum(a.id)}>{a.name}</Button>
    {/each}
    <form class="row-flex" onsubmit={(e) => { e.preventDefault(); createAndAdd(); }}>
      <input class="input grow" placeholder="New album…" bind:value={newAlbum} />
      <Button type="submit" kind="primary" disabled={!newAlbum.trim()}>Create</Button>
    </form>
  </div>
</Sheet>

<style>
  .wrap { height: 100%; display: flex; flex-direction: column; min-height: 0; }
  .selbar { position: absolute; left: 12px; right: 12px; top: 8px; z-index: 6; display: flex; align-items: center; gap: 2px;
    padding: 4px 8px; border-radius: 14px; background: var(--surface); border: 1px solid var(--line); box-shadow: var(--shadow); }
  .selbar.phone { position: fixed; left: 0; right: 0; top: auto; bottom: 0; border-radius: 0; border-width: 1px 0 0; box-shadow: none; padding: 6px 4px calc(6px + var(--safe-bottom)); z-index: 25; }
  .selbar .count { padding: 0 8px; font-size: var(--t-sm); white-space: nowrap; }
  .selbar.phone .count { display: none; }
  .selbar.phone .clear { display: none; }
  .selbar.phone .grow { display: none; }
  .selbar.phone .act { flex: 1; }
  .grow { flex: 1; }
  .clear { width: 36px; height: 36px; border: 0; border-radius: 10px; background: transparent; color: var(--ink); display: grid; place-items: center; cursor: pointer; }
  .act { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 2px; min-width: var(--tap); min-height: var(--tap); padding: 2px 8px; border: 0; border-radius: 10px; background: transparent; color: var(--ink); font: inherit; font-size: var(--t-xs); cursor: pointer; }
  .selbar:not(.phone) .act { flex-direction: row; gap: 6px; min-height: 36px; font-size: var(--t-sm); }
  .act:hover, .clear:hover { background: var(--surface-2); }
  .act.on { color: var(--accent); }
  .act.danger { color: var(--risk); }
  .ctxscrim { position: fixed; inset: 0; z-index: 49; }
  .ctx { position: fixed; z-index: 50; min-width: 200px; padding: 5px; display: grid; background: var(--surface); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow); }
  .ctx button { text-align: left; border: 0; background: none; color: var(--ink); font: inherit; font-size: var(--t-sm); padding: 8px 10px; border-radius: 7px; cursor: pointer; }
  .ctx button:hover { background: var(--surface-2); }
  .ctx .danger { color: var(--risk); }
  .ctx hr { border: 0; border-top: 1px solid var(--line); margin: 4px 0; width: 100%; }
  .menu { display: grid; }
  .menu button { display: flex; align-items: center; gap: 12px; text-align: left; border: 0; border-top: 1px solid var(--line); background: none; color: var(--ink); font: inherit; min-height: var(--tap); cursor: pointer; }
  .menu button:first-child { border-top: 0; }
  .body { flex: 1; position: relative; min-height: 0; display: flex; flex-direction: column; }
  .scroller { flex: 1; overflow-y: auto; position: relative; min-height: 0; touch-action: pan-y; }
  .scroller.railed { scrollbar-width: none; }
  .scroller.railed::-webkit-scrollbar { display: none; }
  .scroller.railed .head { padding-right: 30px; }
  .canvas { position: relative; }
  .barroom { height: 72px; }
  .more { padding: 14px 18px 24px; text-align: center; }
  .head { position: absolute; left: 0; right: 0; display: flex; justify-content: space-between; align-items: baseline; padding: 20px 18px 6px; }
  .head { gap: 6px; align-items: center; }
  .head b { font-size: 15px; }
  .head .n { font-size: var(--t-sm); color: var(--ink-3); }
  /* The day's select circle, left of the date: it slides in on hover (mouse) and while selecting. */
  .day { display: flex; align-items: center; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; text-align: left; }
  .circle { width: 0; height: 24px; margin-right: 0; flex: none; display: grid; place-items: center; position: relative; opacity: 0;
    transition: width 0.15s, margin-right 0.15s, opacity 0.15s; }
  .circle::before { content: ''; position: absolute; left: 1px; top: 3px; width: 18px; height: 18px; box-sizing: border-box; border-radius: 50%; border: 2px solid var(--ink-2); }
  .circle svg { position: relative; width: 12px; height: 12px; fill: none; stroke: var(--accent-ink); stroke-width: 3; stroke-linecap: round; stroke-linejoin: round; opacity: 0; }
  .head.selecting .circle, .day:focus-visible .circle { width: 20px; margin-right: 6px; opacity: 1; }
  .day.all .circle::before { background: var(--accent); border-color: var(--accent); }
  .day.all .circle svg { opacity: 1; }
  .head:not(.selecting) .day { pointer-events: none; cursor: default; }
  @media (hover: hover) {
    .head:hover .circle { width: 20px; margin-right: 6px; opacity: 1; }
    .head:not(.selecting) .circle { pointer-events: auto; cursor: pointer; }
  }
  .tiles { position: absolute; left: 0; right: 0; display: flex; }
  .getting { padding: 10px 14px; border-bottom: 1px solid var(--line); display: flex; flex-direction: column; gap: 4px; background: var(--info-soft); }
  .tips { text-align: left; margin: 8px auto 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
  .empty { height: 100%; display: grid; place-content: center; text-align: center; gap: 6px; color: var(--ink-2); padding: 20px; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
