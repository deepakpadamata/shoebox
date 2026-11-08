<script lang="ts">
  // Albums (U1-10, U1-19): one list for the person's own albums and the shared ones (a Shared
  // pill, a badge for new photos), with Favorites and People as rows on top and Recently Deleted at
  // the bottom. ＋ makes an album or joins one with an invitation.
  import { untrack } from 'svelte';
  import { api, blobUrl } from '../lib/api';
  import { filterAlbums, type AlbumFilter } from '../lib/albumlist';
  import { plural } from '../lib/format';
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
  let seenMenu = ui.albumsMenu;
  $effect(() => { if (ui.albumsMenu !== seenMenu) { seenMenu = ui.albumsMenu; menuOpen = true; } });
  $effect(() => { app.version; app.sharedVersion; untrack(() => { refreshAlbums().catch(() => {}); }); });

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
        <a class="album" href="#/album/{a.id}">
          <div class="cover">
            {#if a.cover}<img src={blobUrl(a.cover, 'thumb', a.coverScope)} alt="" loading="lazy" />{:else}<span><Icon name={a.shared ? 'people' : 'albums'} size={34} /></span>{/if}
            {#if a.unread > 0}<span class="dot" title="{plural(a.unread, 'new photo')}">{a.unread}</span>{/if}
          </div>
          <b>{a.name}{#if a.shared}<span class="pill">Shared</span>{/if}</b>
          <span class="muted small">{plural(a.count, 'photo')}</span>
        </a>
      {/each}
    </div>
    {#if albums.length === 0}<p class="muted">Nothing here yet.</p>{/if}
  {/if}

  <div class="rows end">
    <button class="top" onclick={() => go('deleted')}><Icon name="trash" size={22} /><span class="grow">Recently Deleted</span>{#if app.counts?.deleted}<span class="muted">{app.counts.deleted}</span>{/if}<Icon name="chevron" size={16} /></button>
  </div>
</div>

<Sheet open={menuOpen} title="Albums" onclose={() => (menuOpen = false)} width={360}>
  <div class="menu">
    <button onclick={() => { menuOpen = false; createOpen = true; }}><Icon name="add" size={20} /> New album</button>
    <button onclick={() => { menuOpen = false; joinOpen = true; }}><Icon name="link" size={20} /> Join with an invitation</button>
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
    {#if canScan()}<Button block onclick={scan}>Scan the QR code</Button>{/if}
    <textarea class="input mono" rows="3" placeholder="Paste the invitation here" bind:value={code}></textarea>
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
