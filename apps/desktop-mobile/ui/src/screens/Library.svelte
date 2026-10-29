<script lang="ts">
  // The timeline (M3-06): photos grouped by day, virtualised so 100k photos scroll smoothly,
  // with multi-select and the actions of mockup 2b.
  import { untrack } from 'svelte';
  import { api, type PageCursor, type PhotoPage, type PhotoSummary } from '../lib/api';
  import { dayLabel, plural } from '../lib/format';
  import { buildLayout, visibleRows } from '../lib/timeline';
  import { longPressDrag, pinch, pullRefresh, stepDensity } from '../lib/gestures';
  import { refreshEverything } from '../lib/refresh';
  import { viewport as media } from '../lib/viewport.svelte';
  import { app, fail, toast } from '../lib/store.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import { go } from '../lib/route.svelte';
  import Tile from '../components/Tile.svelte';
  import DateRail from '../components/DateRail.svelte';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';
  import ShareSheet from '../components/ShareSheet.svelte';
  import Viewer from './Viewer.svelte';

  interface Props { source: 'all' | 'favorites' | 'album'; album?: string; }
  let { source, album }: Props = $props();

  // An album of your own can be shared: a shared album with its photos, kept linked (A-097).
  let sharedAs = $state<string | null>(null);
  let shareAsk = $state(0);
  let sharing = $state(false);
  $effect(() => {
    if (source !== 'album' || !album) return;
    app.version;
    untrack(() => { api.albums().then((all) => (sharedAs = all.find((a) => a.id === album)?.shared_as ?? null)).catch(() => {}); });
  });
  async function startShare() {
    try {
      const check = await api.albumShareCheck(album!);
      if (check.with_location > 0) shareAsk = check.with_location;
      else await share('remove');
    } catch (e) { fail(e); }
  }
  async function share(location: string) {
    sharing = true;
    try {
      const s = await api.albumShare(album!, location);
      shareAsk = 0;
      go(`shared/${s.id}`);
    } catch (e) { fail(e); } finally { sharing = false; }
  }

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
  function setDensity(scale: number) {
    density = stepDensity(minTile, scale, DENSITIES);
    localStorage.setItem('density', String(density));
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

  const layout = $derived(buildLayout(items, { width, minTile, gap: GAP, headHeight: HEAD }));
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
    const key = `${source}/${album ?? ''}`;
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
    if ((e.key === 'Delete' || e.key === 'Backspace') && selecting && !(e.target instanceof HTMLInputElement)) removeSelected();
    if ((e.metaKey || e.ctrlKey) && e.key === 'a' && !(e.target instanceof HTMLInputElement)) {
      e.preventDefault();
      selectAll();
    }
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

{#snippet glyph(name: 'close' | 'share' | 'heart' | 'album' | 'keep' | 'unalbum' | 'trash')}
  <svg viewBox="0 0 24 24" aria-hidden="true">
    {#if name === 'close'}<path d="M6 6l12 12M18 6L6 18" />
    {:else if name === 'share'}<path d="M12 3.5v11M8 7.5l4-4 4 4M7 10.5H5.5v9h13v-9H17" />
    {:else if name === 'heart'}<path class="fill" d="M12 20s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7.3a4.3 4.3 0 0 1 7.5 2.5C19.5 15.4 12 20 12 20z" />
    {:else if name === 'album'}<rect x="3.5" y="3.5" width="17" height="17" rx="4" /><path d="M12 8v8M8 12h8" />
    {:else if name === 'keep'}<path d="M12 4v10M7.5 9.5L12 14l4.5-4.5M5 19.5h14" />
    {:else if name === 'unalbum'}<rect x="3.5" y="3.5" width="17" height="17" rx="4" /><path d="M8 12h8" />
    {:else}<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5M10 11v5M14 11v5" />{/if}
  </svg>
{/snippet}

<div class="wrap">
  {#if firstSync}
    <div class="getting" role="status">
      <b>Getting your photos</b>
      <span class="muted small">{(wantThumbs.length - missingThumbs).toLocaleString('en')} of {wantThumbs.length.toLocaleString('en')} previews are on this device.{gettingEta}</span>
      <ProgressBar total={wantThumbs.length} segments={[{ value: wantThumbs.length - missingThumbs, tone: 'info' }]} label="Previews downloaded" />
    </div>
  {/if}
  {#if source === 'album' && album}
    <div class="sharebar">
      {#if sharedAs}
        <span class="grow small">Shared with others. Photos you add here are shared too.</span>
        <Button size="sm" onclick={() => go(`shared/${sharedAs}`)}>Open shared album</Button>
      {:else}
        <span class="grow small muted">Only on your devices.</span>
        <Button size="sm" disabled={sharing} onclick={startShare}>{sharing ? 'Sharing…' : 'Share this album'}</Button>
      {/if}
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
    {:else if items.length === 0}
      <div class="empty">
        <b>{source === 'favorites' ? 'No favorites yet' : source === 'album' ? 'This album is empty' : 'No photos yet'}</b>
        <span class="muted">{source === 'all' ? 'Bring photos in from a folder, or add another device to start backing up.' : source === 'favorites' ? 'Tap the heart on a photo to find it here quickly.' : 'Select photos in your library and add them here.'}</span>
        {#if source === 'all'}
          <div class="row-flex" style="justify-content:center;margin-top:10px">
            {#if media.mobile}
              <Button kind="primary" onclick={() => go(app.backup && app.backup.permission !== 'unavailable' ? 'settings' : 'devices')}>{app.backup && app.backup.permission !== 'unavailable' ? 'Turn on camera backup' : 'Add a device'}</Button>
            {:else}
              <Button kind="primary" onclick={() => app.importRequest++}>Import photos</Button>
              <Button onclick={() => go('devices')}>Add a device</Button>
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
              <button class="day" class:all aria-pressed={all} aria-label="Select {dayLabel(row.day)}" title={all ? 'Clear this day' : 'Select this day'}
                onclick={() => toggleDay(row.from, row.count)}>
                <span class="circle"><svg viewBox="0 0 24 24"><path d="M7 12.5l3.2 3.2L17 9" /></svg></span>
                <b>{dayLabel(row.day)}</b>
              </button>
              <span class="grow"></span>
              <span class="n">{row.count.toLocaleString('en')}</span>
            </div>
          {:else}
            <div class="tiles" style="top:{row.top}px;height:{row.height}px;gap:{GAP}px">
              {#each items.slice(row.from, row.to) as p, k (p.id)}
                <Tile index={row.from + k} id={p.id} size={layout.tile} favorite={p.favorite} video={p.is_video} copies={p.copies} {target}
                  takenAt={p.taken_at} hasThumb={p.has_thumb} hasPoster={p.has_poster} hasOriginal={p.has_original} backedUp={p.backed_up} selected={selectedSet.has(p.id)} {selecting} onclick={(e) => click(row.from + k, e)} />
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
    <!-- Floats over the grid, so starting a selection moves nothing under the finger. -->
    <div class="selbar" role="toolbar" aria-label="{selected.length} selected">
      <button class="ic" title="Clear selection" aria-label="Clear selection" onclick={() => (selected = [])}>{@render glyph('close')}</button>
      <b class="count">{selected.length.toLocaleString('en')}</b>
      <span class="sep"></span>
      <button class="ic" bind:this={shareButton} title="Share" aria-label="Share" onclick={() => (shareSheet = true)}>{@render glyph('share')}</button>
      <button class="ic" class:on={allFavorite} title={allFavorite ? 'Unfavorite' : 'Favorite'} aria-label={allFavorite ? 'Unfavorite' : 'Favorite'}
        onclick={() => act(() => api.setFavorite(selected, !allFavorite))}>{@render glyph('heart')}</button>
      <button class="ic" title="Add to album" aria-label="Add to album" onclick={openAlbumSheet}>{@render glyph('album')}</button>
      <button class="ic" title="Keep on this device" aria-label="Keep on this device"
        onclick={() => act(() => Promise.all(selected.map((id) => api.pinOriginal(id, true))), 'Originals will be kept on this device')}>{@render glyph('keep')}</button>
      {#if source === 'album' && album}
        <button class="ic" title="Remove from album" aria-label="Remove from album"
          onclick={() => act(() => api.albumRemove(album, selected), 'Removed from the album').then(() => (selected = []))}>{@render glyph('unalbum')}</button>
      {/if}
      <button class="ic danger" title="Delete from all devices" aria-label="Delete from all devices" onclick={removeSelected}>{@render glyph('trash')}</button>
    </div>
  {/if}
  {#if showRail}
    <DateRail {layout} {scrollTop} {viewport} onseek={(top) => { if (scroller) scroller.scrollTop = top; }} />
  {/if}
  </div>
</div>

{#if viewing !== null && items[viewing]}
  <Viewer {items} index={viewing} onclose={() => (viewing = null)} onmove={jump} />
{/if}

<Sheet open={shareAsk > 0} title="Share the location?" onclose={() => (shareAsk = 0)} width={420}>
  <div class="stack">
    <p>{plural(shareAsk, 'photo')} in this album {shareAsk === 1 ? 'was' : 'were'} taken at a known place. People you share it with would see where. Your choice also applies to photos you add to the album later.</p>
    <Button kind="primary" block disabled={sharing} onclick={() => share('remove')}>Remove the location</Button>
    <Button block disabled={sharing} onclick={() => share('keep')}>Keep the location</Button>
  </div>
</Sheet>

<ShareSheet open={shareSheet} ids={selected} anchor={() => shareButton?.getBoundingClientRect()} onclose={() => (shareSheet = false)}
  ondone={() => { shareSheet = false; selected = []; }} />

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
  .sharebar { display: flex; gap: 8px; align-items: center; padding: 8px 18px; border-bottom: 1px solid var(--line); }
  .selbar { position: absolute; left: 50%; bottom: 14px; transform: translateX(-50%); z-index: 6; display: flex; align-items: center; gap: 2px;
    padding: 4px 6px; border-radius: 16px; background: var(--surface); border: 1px solid var(--line); box-shadow: var(--shadow); }
  .selbar .count { min-width: 22px; padding: 0 6px 0 2px; font-size: 14px; font-variant-numeric: tabular-nums; }
  .selbar .sep { width: 1px; height: 22px; background: var(--line); margin: 0 4px; }
  .ic { width: 40px; height: 40px; border: 0; border-radius: 12px; background: transparent; color: var(--ink); display: grid; place-items: center; cursor: pointer; padding: 0; }
  .ic:hover { background: var(--surface-2); }
  .ic svg { width: 21px; height: 21px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .ic.on { color: var(--accent); }
  .ic.on .fill { fill: currentColor; }
  .ic.danger { color: var(--risk); }
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
  .head .n { font-size: 12px; color: var(--ink-3); }
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
  .empty { height: 100%; display: grid; place-content: center; text-align: center; gap: 6px; color: var(--ink-2); padding: 20px; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
