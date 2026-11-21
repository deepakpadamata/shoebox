<script lang="ts">
  // Albums (U1-10, U1-19): one list for the person's own albums and the shared ones (a Shared
  // pill, a badge for new photos), with Favorites and People as rows on top and Recently Deleted at
  // the bottom. ＋ makes an album or joins one with an invitation.
  import { onMount, untrack } from 'svelte';
  import { api, blobUrl } from '../lib/api';
  import { deletable, filterAlbums, type AlbumEntry, type AlbumFilter } from '../lib/albumlist';
  import { dateRange, plural } from '../lib/format';
  import { app, fail, refreshAlbums, toast, ui } from '../lib/store.svelte';
  import { go } from '../lib/route.svelte';
  import { canScan, scanQr } from '../lib/scan';
  import { pullRefresh } from '../lib/gestures';
  import { refreshEverything } from '../lib/refresh';
  import { viewport } from '../lib/viewport.svelte';
  import Button from '../components/Button.svelte';
  import Icon from '../components/Icon.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Sheet from '../components/Sheet.svelte';

  let filter = $state<AlbumFilter>('all');
  const albums = $derived(filterAlbums(app.albums, filter));
  let menuOpen = $state(false);
  let createOpen = $state(false);
  let joinOpen = $state(false);
  let name = $state('');
  let code = $state('');
  let busy = $state(false);
  // Smart albums: a name and the search that fills them.
  let smartOpen = $state(false);
  let smartName = $state('');
  let smartQuery = $state('');
  // Choosing albums to delete several at once.
  let selecting = $state(false);
  let chosen = $state<string[]>([]);
  let confirmOpen = $state(false);
  const chosenAlbums = $derived(app.albums.filter((a) => chosen.includes(a.id)));
  const chosenPhotos = $derived(chosenAlbums.filter((a) => a.personal).reduce((n, a) => n + a.count, 0));
  function stopSelecting() { selecting = false; chosen = []; }
  function toggle(a: AlbumEntry) {
    if (!deletable(a)) return;
    chosen = chosen.includes(a.id) ? chosen.filter((x) => x !== a.id) : [...chosen, a.id];
  }
  const subtitle = (a: AlbumEntry) =>
    a.smart !== undefined ? 'Smart album' : [plural(a.count, 'photo'), dateRange(a.firstMs, a.lastMs)].filter(Boolean).join(' · ');
  // A cover whose picture is not there (yet) shows the album's icon, not a broken image; it is
  // tried again when something changes (a thumbnail may have arrived).
  let broken = $state<Record<string, boolean>>({});
  $effect(() => { app.version; app.albumsVersion; app.holdingsVersion; app.sharedVersion; untrack(() => { broken = {}; }); });
  let seenMenu = ui.albumsMenu;
  $effect(() => { if (ui.albumsMenu !== seenMenu) { seenMenu = ui.albumsMenu; menuOpen = true; } });
  // The store keeps the list current as albums change; this only fetches it on opening.
  onMount(() => { refreshAlbums().catch(() => {}); });

  async function create() {
    busy = true;
    try {
      const a = await api.createAlbum(name);
      name = '';
      createOpen = false;
      await refreshAlbums();
      go(`album/${a.id}`);
    } catch (e) { fail(e); } finally { busy = false; }
  }
  async function createSmart() {
    busy = true;
    try {
      const a = await api.saveSmartAlbum(smartName, smartQuery);
      smartName = smartQuery = '';
      smartOpen = false;
      await refreshAlbums();
      go(`album/${a.id}`);
    } catch (e) { fail(e); } finally { busy = false; }
  }
  // One at a time, so a failure (a web link the service could not stop) leaves the rest done and says which.
  async function deleteChosen() {
    busy = true;
    let done = 0;
    const failed: string[] = [];
    for (const a of chosenAlbums) {
      try {
        if (a.smart !== undefined) await api.deleteSmartAlbum(a.id);
        else await api.deleteAlbum(a.id);
        done++;
      } catch (e) {
        failed.push(`${a.name}: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
    busy = false;
    confirmOpen = false;
    stopSelecting();
    await refreshAlbums().catch(() => {});
    if (failed.length) fail(new Error(`${done ? `Deleted ${plural(done, 'album')}. ` : ''}Not deleted: ${failed.join('; ')}`));
    else toast(`Deleted ${plural(done, 'album')}`);
  }
  async function join() {
    busy = true;
    try {
      const a = await api.sharedJoin(code.trim());
      code = '';
      joinOpen = false;
      toast(`You joined “${a.name}”`);
      await refreshAlbums();
      go(`album/${a.id}`);
    } catch (e) { fail(e); } finally { busy = false; }
  }
  async function pasteCode() {
    try {
      const text = await navigator.clipboard.readText();
      if (text.trim()) code = text.trim();
      else toast('The clipboard is empty');
    } catch { toast('Press and hold in the box below to paste'); }
  }
  async function scan() {
    try {
      const text = await scanQr();
      if (text) code = text.trim();
    } catch (e) { fail(e); }
  }
</script>

<div class="page" use:pullRefresh={{ onRefresh: refreshEverything }}>
  {#if !viewport.mobile}
    <div class="head">
      {#if app.albums.some((a) => a.shared)}
        <div class="seg"><Segmented bind:value={filter} options={[{ value: 'all', label: 'All' }, { value: 'mine', label: 'Mine' }, { value: 'shared', label: 'Shared' }]} /></div>
      {/if}
      <span class="grow"></span>
      {#if app.albums.some(deletable)}<Button size="sm" kind={selecting ? 'primary' : 'default'} onclick={() => (selecting ? stopSelecting() : (selecting = true))}>{selecting ? 'Done' : 'Select'}</Button>{/if}
      <button class="plus" aria-label="New album or join one" onclick={() => (menuOpen = true)}><Icon name="add" size={22} /></button>
    </div>
  {/if}

  <div class="rows">
    <button class="top" onclick={() => go('favorites')}><Icon name="heart" size={22} /><span class="grow">Favorites</span><span class="muted">{app.counts?.favorites ?? 0}</span><Icon name="chevron" size={16} /></button>
    <button class="top" onclick={() => go('people')}><Icon name="people" size={22} /><span class="grow">People</span><Icon name="chevron" size={16} /></button>
  </div>

  {#if app.albums.length === 0}
    <div class="empty">
      <b>No albums yet</b>
      <p class="muted">Group photos you want to find again, or share with someone. Select photos in Photos and choose Add to album, or start one here.</p>
      <div class="row-flex"><Button kind="primary" onclick={() => (createOpen = true)}>New album</Button><Button onclick={() => (joinOpen = true)}>Join with an invitation</Button></div>
    </div>
  {:else}
    <div class="grid">
      {#each albums as a (a.id)}
        {#snippet tile()}
          <div class="cover">
            {#if a.cover && !broken[a.id + a.cover]}<img src={blobUrl(a.cover, 'thumb', a.coverScope)} alt="" loading="lazy" onerror={() => (broken[a.id + a.cover] = true)} />{:else}<span><Icon name={a.smart !== undefined ? 'search' : a.shared ? 'people' : 'albums'} size={34} /></span>{/if}
            {#if a.unread > 0 && !selecting}<span class="dot" title="{plural(a.unread, 'new photo')}">{a.unread}</span>{/if}
            {#if selecting && deletable(a)}<span class="tick" class:on={chosen.includes(a.id)}>{#if chosen.includes(a.id)}<Icon name="check" size={16} />{/if}</span>{/if}
          </div>
          <b>{a.name}{#if a.shared}<span class="pill">Shared</span>{/if}</b>
          <span class="muted small">{subtitle(a)}</span>
        {/snippet}
        {#if selecting}
          <button class="album pick" class:off={!deletable(a)} disabled={!deletable(a)} aria-pressed={chosen.includes(a.id)} title={deletable(a) ? '' : 'Shared albums are left, not deleted'} onclick={() => toggle(a)}>{@render tile()}</button>
        {:else}
          <a class="album" href="#/album/{a.id}">{@render tile()}</a>
        {/if}
      {/each}
    </div>
    {#if albums.length === 0}<p class="muted">Nothing here yet.</p>{/if}
  {/if}

  {#if selecting}
    <div class="selbar">
      <span class="grow">{chosen.length ? plural(chosen.length, 'album') + ' selected' : 'Choose albums to delete'}</span>
      <Button onclick={stopSelecting}>Cancel</Button>
      <Button kind="danger" disabled={!chosen.length} onclick={() => (confirmOpen = true)}><Icon name="trash" size={16} /> Delete</Button>
    </div>
  {/if}

  <div class="rows end">
    <button class="top" onclick={() => go('deleted')}><Icon name="trash" size={22} /><span class="grow">Recently Deleted</span>{#if app.counts?.deleted}<span class="muted">{app.counts.deleted}</span>{/if}<Icon name="chevron" size={16} /></button>
  </div>
</div>

<Sheet open={menuOpen} title="Albums" onclose={() => (menuOpen = false)} width={360}>
  <div class="menu">
    <button onclick={() => { menuOpen = false; createOpen = true; }}><Icon name="add" size={20} /> New album</button>
    <button onclick={() => { menuOpen = false; smartOpen = true; }}><Icon name="search" size={20} /> New smart album</button>
    <button onclick={() => { menuOpen = false; joinOpen = true; }}><Icon name="link" size={20} /> Join with an invitation</button>
    {#if app.albums.some(deletable)}<button onclick={() => { menuOpen = false; selecting = true; }}><Icon name="check" size={20} /> Select albums</button>{/if}
  </div>
</Sheet>

<Sheet open={smartOpen} title="New smart album" onclose={() => (smartOpen = false)} width={400}>
  <form class="stack" onsubmit={(e) => { e.preventDefault(); createSmart(); }}>
    <p class="muted small">It shows whatever this search finds, so new photos join it by themselves. Try a person, a place, a year or a thing: “beach 2024”, “screenshots”.</p>
    <input class="input" placeholder="Album name" aria-label="Album name" bind:value={smartName} />
    <input class="input" placeholder="Search for…" aria-label="Search" bind:value={smartQuery} />
    <Button type="submit" kind="primary" block disabled={busy || !smartName.trim() || !smartQuery.trim()}>Create smart album</Button>
  </form>
</Sheet>

<Sheet open={confirmOpen} title="Delete {plural(chosen.length, 'album')}?" onclose={() => (confirmOpen = false)} width={400}>
  <div class="stack">
    <p class="muted">{chosenAlbums.map((a) => a.name).join(', ')}</p>
    <p class="muted">They go from all your devices.{chosenPhotos ? ` Their ${plural(chosenPhotos, 'photo')} stay in your library.` : ''}</p>
    {#if chosenAlbums.some((a) => a.sharedId)}<p class="muted small">Shared albums they were shared as stay for the people in them.</p>{/if}
    {#if chosenAlbums.some((a) => a.personal)}<p class="muted small">Any web links to them stop working.</p>{/if}
    <Button kind="danger" block disabled={busy} onclick={deleteChosen}>{busy ? 'Deleting…' : `Delete ${plural(chosen.length, 'album')}`}</Button>
    <Button kind="ghost" block onclick={() => (confirmOpen = false)}>Cancel</Button>
  </div>
</Sheet>

<Sheet open={createOpen} title="New album" onclose={() => (createOpen = false)} width={380}>
  <form class="stack" onsubmit={(e) => { e.preventDefault(); create(); }}>
    <input class="input" placeholder="Album name" aria-label="Album name" bind:value={name} />
    <Button type="submit" kind="primary" block disabled={busy || !name.trim()}>Create album</Button>
  </form>
</Sheet>

<Sheet open={joinOpen} title="Join a shared album" onclose={() => (joinOpen = false)} width={440}>
  <div class="stack">
    <p class="muted small">Ask the owner to send you the invitation (a message, or a QR code). Both of you need to be online while you join.</p>
    <div class="row-flex">
      {#if canScan()}<Button block onclick={scan}><Icon name="link" size={18} /> Scan the QR code</Button>{/if}
      <Button block onclick={pasteCode}>Paste the invitation</Button>
    </div>
    <textarea class="input mono" rows="3" placeholder="Or paste it here" bind:value={code}></textarea>
    <Button kind="primary" block disabled={busy || !code.trim()} onclick={join}>{busy ? 'Joining…' : 'Join'}</Button>
  </div>
</Sheet>

<style>
  .page { padding: 6px 16px calc(18px + var(--safe-bottom)); overflow: auto; height: 100%; }
  .head { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; }
  .seg { width: 260px; }
  .grow { flex: 1; }
  .plus { width: 40px; height: 40px; border-radius: 50%; border: 0; background: var(--accent-soft); color: var(--accent); display: grid; place-items: center; cursor: pointer; }
  .rows { display: grid; border: 1px solid var(--line); border-radius: var(--radius); overflow: hidden; background: var(--surface); }
  .rows.end { margin-top: 18px; }
  .top { display: flex; align-items: center; gap: 12px; min-height: var(--tap); padding: 0 14px; border: 0; background: none; font: inherit; font-size: var(--t-md); color: var(--ink); cursor: pointer; text-align: left; }
  .top + .top { border-top: 1px solid var(--line); }
  .top:hover { background: var(--surface-2); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 16px 12px; margin-top: 18px; }
  @media (min-width: 721px) { .page { padding-top: 14px; padding-left: 18px; padding-right: 18px; } }
  @media (max-width: 720px) { .grid { grid-template-columns: repeat(2, 1fr); } }
  .album { text-decoration: none; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .pick { border: 0; background: none; padding: 0; font: inherit; color: inherit; text-align: left; cursor: pointer; }
  .pick.off { opacity: .45; cursor: default; }
  .tick { position: absolute; left: 8px; top: 8px; width: 24px; height: 24px; border-radius: 50%; border: 2px solid #fff; background: rgba(0,0,0,.25); display: grid; place-items: center; color: var(--accent-ink); }
  .tick.on { background: var(--accent); border-color: var(--accent); }
  .pick[aria-pressed='true'] .cover { outline: 3px solid var(--accent); outline-offset: -3px; }
  .selbar { position: sticky; bottom: 0; margin-top: 14px; display: flex; align-items: center; gap: 8px; padding: 10px 12px; border-radius: var(--radius); background: var(--surface); border: 1px solid var(--line); box-shadow: 0 6px 20px rgba(0,0,0,.18); }
  .album b { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cover { position: relative; aspect-ratio: 1; border-radius: 12px; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; color: var(--ink-3); margin-bottom: 6px; }
  .cover img { width: 100%; height: 100%; object-fit: cover; }
  .dot { position: absolute; right: 8px; top: 8px; min-width: 20px; height: 20px; padding: 0 6px; border-radius: 10px; background: var(--accent); color: var(--accent-ink); font-size: var(--t-xs); font-weight: 700; display: grid; place-items: center; }
  .pill { margin-left: 6px; font-size: var(--t-xs); font-weight: 600; padding: 1px 7px; border-radius: 999px; background: var(--accent-soft); color: var(--accent); vertical-align: 2px; }
  .empty { margin: 28px 0; display: grid; gap: 8px; max-width: 460px; }
  .empty p { margin: 0; }
  .menu { display: grid; }
  .menu button { display: flex; align-items: center; gap: 12px; text-align: left; border: 0; border-top: 1px solid var(--line); background: none; color: var(--ink); font: inherit; min-height: var(--tap); cursor: pointer; }
  .menu button:first-child { border-top: 0; }
  .input { border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px 12px; color: var(--ink); font: inherit; width: 100%; }
</style>
