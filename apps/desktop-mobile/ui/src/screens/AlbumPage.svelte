<script lang="ts">
  // One album page (U1-19) for every album: cover, name, who is in it, Add photos, Share and ⋯.
  // A personal album shows its photos; once it is shared (or when it is someone else's), the same
  // page shows the shared album, with Photos, Activity and People.
  import { onMount } from 'svelte';
  import { api, blobUrl, type PhotoSummary, type SharedAlbumInfo } from '../lib/api';
  import { findAlbum } from '../lib/albumlist';
  import { plural } from '../lib/format';
  import { app, fail, refreshAlbums, toast } from '../lib/store.svelte';
  import { go } from '../lib/route.svelte';
  import AddToAlbumSheet from '../components/AddToAlbumSheet.svelte';
  import AlbumShareSheet from '../components/AlbumShareSheet.svelte';
  import Button from '../components/Button.svelte';
  import Icon from '../components/Icon.svelte';
  import Sheet from '../components/Sheet.svelte';
  import WebLinkSheet from '../components/WebLinkSheet.svelte';
  import Library from './Library.svelte';
  import SharedAlbum from './SharedAlbum.svelte';

  let { id }: { id: string } = $props();
  const entry = $derived(findAlbum(app.albums, id));
  let loaded = $state(app.albums.length > 0);
  onMount(() => { refreshAlbums().then(() => (loaded = true)).catch(() => (loaded = true)); });

  let sharedInfo = $state<SharedAlbumInfo | null>(null);
  const role = $derived(sharedInfo?.role ?? entry?.role ?? (entry?.personal ? 'owner' : 'viewer'));
  const canAdd = $derived(role !== 'viewer' && app.state?.role !== 'viewer');
  const owner = $derived(role === 'owner');
  let settingsOpen = $state(false);
  let addOpen = $state(false);
  let shareOpen = $state(false);
  let linkOpen = $state(false);
  let menuOpen = $state(false);
  let renameOpen = $state(false);
  let renameTo = $state('');
  let coverOpen = $state(false);
  let coverChoices = $state<PhotoSummary[]>([]);

  async function rename() {
    try {
      await api.renameAlbum(entry!.id, renameTo);
      renameOpen = false;
      await refreshAlbums();
    } catch (e) { fail(e); }
  }
  async function openCover() {
    coverOpen = true;
    coverChoices = (await api.albumPage(entry!.id, null, 300).catch(() => ({ items: [], next: null }))).items;
  }
  async function setCover(photo: string) {
    try {
      await api.setAlbumCover(entry!.id, photo);
      coverOpen = false;
      toast('Cover changed');
      await refreshAlbums();
    } catch (e) { fail(e); }
  }
  // Albums added through a shared page refresh the list when their photos arrive.
  const sub = $derived(entry ? `${plural(entry.count, 'photo')}${entry.shared ? ' · Shared' : ''}` : '');
</script>

{#if entry}
  <div class="album">
    <header class="ahead">
      <div class="cover">{#if entry.cover}<img src={blobUrl(entry.cover, 'thumb', entry.coverScope)} alt="" />{:else}<Icon name={entry.shared ? 'people' : 'albums'} size={30} />{/if}</div>
      <div class="who">
        <h2>{entry.name}</h2>
        <span class="muted small">{sub}</span>
      </div>
      <div class="acts">
        {#if canAdd}<Button kind="primary" size="sm" onclick={() => (addOpen = true)}><Icon name="add" size={16} /> Add photos</Button>{/if}
        {#if entry.personal || owner}<Button size="sm" onclick={() => (shareOpen = true)}><Icon name="share" size={16} /> Share</Button>{/if}
        <button class="more" aria-label="More" onclick={() => (menuOpen = true)}><Icon name="more" size={22} /></button>
      </div>
    </header>
    <div class="body">
      {#if entry.sharedId}
        {#key entry.sharedId}<SharedAlbum album={entry.sharedId} bind:settingsOpen bind:info={sharedInfo} />{/key}
      {:else}
        {#key entry.id}<Library source="album" album={entry.id} />{/key}
      {/if}
    </div>
  </div>

  <Sheet open={menuOpen} title={entry.name} onclose={() => (menuOpen = false)} width={340}>
    <div class="menu">
      {#if entry.personal}
        <button onclick={() => { menuOpen = false; renameTo = entry.name; renameOpen = true; }}><Icon name="edit" size={20} /> Rename</button>
        {#if entry.count > 0}<button onclick={() => { menuOpen = false; openCover(); }}><Icon name="photos" size={20} /> Set cover</button>{/if}
      {/if}
      {#if entry.sharedId}
        <button onclick={() => { menuOpen = false; settingsOpen = true; }}><Icon name="settings" size={20} /> Album settings</button>
        <button class="danger" onclick={() => { menuOpen = false; settingsOpen = true; }}><Icon name="close" size={20} /> Leave album</button>
      {/if}
    </div>
  </Sheet>

  <AddToAlbumSheet open={addOpen} name={entry.name} personal={entry.personal ? entry.id : undefined} shared={entry.sharedId} onclose={() => (addOpen = false)} ondone={() => { addOpen = false; refreshAlbums(); }} />
  <AlbumShareSheet open={shareOpen} name={entry.name} personal={entry.personal ? entry.id : undefined} shared={entry.sharedId} {owner} onclose={() => (shareOpen = false)} onweblink={() => (linkOpen = true)} />
  {#if entry.personal}<WebLinkSheet open={linkOpen} album={entry.id} onclose={() => (linkOpen = false)} />{/if}

  <Sheet open={renameOpen} title="Rename album" onclose={() => (renameOpen = false)} width={380}>
    <form class="stack" onsubmit={(e) => { e.preventDefault(); rename(); }}>
      <input class="input" bind:value={renameTo} aria-label="Album name" />
      <Button type="submit" kind="primary" block disabled={!renameTo.trim()}>Rename</Button>
    </form>
  </Sheet>

  <Sheet open={coverOpen} title="Choose a cover" onclose={() => (coverOpen = false)} width={520}>
    <div class="pick">
      {#each coverChoices as p (p.id)}
        <button class="ptile" onclick={() => setCover(p.id)} aria-label="Use this photo">{#if p.has_thumb && !p.is_video}<img src={blobUrl(p.id, 'thumb')} alt="" loading="lazy" />{/if}</button>
      {/each}
    </div>
  </Sheet>
{:else if loaded}
  <div class="gone"><p class="muted">This album is not on this device any more.</p><Button onclick={() => go('albums')}>Back to albums</Button></div>
{:else}
  <div class="gone muted">Loading…</div>
{/if}

<style>
  .album { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .ahead { flex: none; display: flex; align-items: center; gap: 12px; padding: 10px 18px; flex-wrap: wrap; }
  .cover { width: 56px; height: 56px; border-radius: 12px; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; color: var(--ink-3); flex: none; }
  .cover img { width: 100%; height: 100%; object-fit: cover; }
  .who { min-width: 0; flex: 1; display: grid; }
  h2 { margin: 0; font-size: var(--t-xl); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .acts { display: flex; align-items: center; gap: 8px; }
  .more { width: 40px; height: 40px; border: 0; border-radius: 50%; background: var(--surface-2); color: var(--ink); display: grid; place-items: center; cursor: pointer; }
  .body { flex: 1; min-height: 0; overflow: hidden; }
  .gone { padding: 24px; display: grid; gap: 10px; justify-items: start; }
  .menu { display: grid; }
  .menu button { display: flex; align-items: center; gap: 12px; text-align: left; border: 0; border-top: 1px solid var(--line); background: none; color: var(--ink); font: inherit; min-height: var(--tap); cursor: pointer; }
  .menu button:first-child { border-top: 0; }
  .menu .danger { color: var(--risk); }
  .pick { display: grid; grid-template-columns: repeat(auto-fill, minmax(84px, 1fr)); gap: 3px; max-height: 50vh; overflow: auto; }
  .ptile { aspect-ratio: 1; border: 0; padding: 0; background: var(--surface-2); cursor: pointer; overflow: hidden; }
  .ptile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; width: 100%; }
</style>
