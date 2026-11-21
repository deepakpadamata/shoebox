<script lang="ts">
  // One album page (U1-19) for every album: cover, name, who is in it, Add photos, Share and ⋯.
  // A personal album shows its photos; once it is shared (or when it is someone else's), the same
  // page shows the shared album, with Photos, Activity and People. A smart album shows what its
  // search finds now.
  import { onMount } from 'svelte';
  import { api, blobUrl, type PhotoSummary, type SharedAlbumInfo } from '../lib/api';
  import { findAlbum } from '../lib/albumlist';
  import { bytes, dateRange, plural } from '../lib/format';
  import { canPick, isHandheld, pickFolder } from '../lib/pick';
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
  let deleteOpen = $state(false);
  let deleting = $state(false);
  let coverChoices = $state<PhotoSummary[]>([]);
  let noteOpen = $state(false);
  let noteText = $state('');
  let queryOpen = $state(false);
  let queryText = $state('');
  let exportOpen = $state(false);
  let exportDir = $state('');
  let exporting = $state(false);
  const smart = $derived(entry?.smart !== undefined);
  // A cover that failed to load (not on this device yet) shows the album icon, as on Albums.
  let brokenCover = $state('');

  async function rename() {
    try {
      if (smart) await api.saveSmartAlbum(renameTo, entry!.smart!, entry!.id);
      else await api.renameAlbum(entry!.id, renameTo);
      renameOpen = false;
      await refreshAlbums();
    } catch (e) { fail(e); }
  }
  async function remove() {
    deleting = true;
    try {
      const name = entry!.name;
      if (smart) await api.deleteSmartAlbum(entry!.id);
      else await api.deleteAlbum(entry!.id);
      deleteOpen = false;
      // Leave first: refreshing while this page still shows the album would pull it out from under it.
      go('albums');
      toast(`Deleted “${name}”`);
      refreshAlbums().catch(fail);
    } catch (e) { fail(e); } finally { deleting = false; }
  }
  async function saveNote() {
    try {
      await api.setAlbumNote(entry!.id, noteText);
      noteOpen = false;
      await refreshAlbums();
    } catch (e) { fail(e); }
  }
  async function saveQuery() {
    try {
      await api.saveSmartAlbum(entry!.name, queryText, entry!.id);
      queryOpen = false;
      await refreshAlbums();
    } catch (e) { fail(e); }
  }
  async function exportTo(dest: string) {
    exporting = true;
    try {
      const r = await api.exportAlbum(entry!.id, dest);
      exportOpen = false;
      toast(`Exported ${plural(r.written, 'photo')} (${bytes(r.bytes)}) to ${r.folder}${r.missing ? `. ${plural(r.missing, 'original')} not on this device were left out` : ''}`);
    } catch (e) { fail(e); } finally { exporting = false; }
  }
  async function chooseAndExport() {
    const d = await pickFolder('Export the album into…').catch((e) => { fail(e); return null; });
    if (d) await exportTo(d);
  }
  // On a phone there is no folder to pick: the album's photos go to the system share sheet
  // (Save to Files, another app), fetched from another device where they are not here.
  async function shareAll() {
    exporting = true;
    try {
      const ids: string[] = [];
      let cursor = null as Awaited<ReturnType<typeof api.albumPage>>['next'];
      do {
        const p = await api.albumPage(entry!.id, cursor, 500);
        ids.push(...p.items.map((x) => x.id));
        cursor = p.next;
      } while (cursor);
      if (ids.length) await api.shareFiles(ids);
      exportOpen = false;
    } catch (e) { fail(e); } finally { exporting = false; }
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
  const sub = $derived(
    !entry ? '' : entry.smart !== undefined ? `Smart album · finds “${entry.smart}”`
      : [plural(entry.count, 'photo'), dateRange(entry.firstMs, entry.lastMs), entry.shared ? 'Shared' : ''].filter(Boolean).join(' · '),
  );
</script>

{#if entry}
  <div class="album">
    <header class="ahead">
      <div class="cover">{#if entry.cover && brokenCover !== entry.cover}<img src={blobUrl(entry.cover, 'thumb', entry.coverScope)} alt="" onerror={() => (brokenCover = entry.cover ?? '')} />{:else}<Icon name={smart ? 'search' : entry.shared ? 'people' : 'albums'} size={30} />{/if}</div>
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
    {#if entry.note}<button class="note" title="Edit the description" onclick={() => { noteText = entry.note; noteOpen = true; }}>{entry.note}</button>{/if}
    <div class="body">
      {#if smart}
        {#key entry.smart}<Library source="search" query={entry.smart} />{/key}
      {:else if entry.sharedId}
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
        <button onclick={() => { menuOpen = false; noteText = entry.note; noteOpen = true; }}><Icon name="info" size={20} /> {entry.note ? 'Edit description' : 'Add a description'}</button>
        {#if entry.count > 0}<button onclick={() => { menuOpen = false; exportDir = ''; exportOpen = true; }}><Icon name="download" size={20} /> {isHandheld() ? 'Save or send all photos' : 'Export…'}</button>{/if}
        <button class="danger" onclick={() => { menuOpen = false; deleteOpen = true; }}><Icon name="trash" size={20} /> Delete album</button>
      {/if}
      {#if smart}
        <button onclick={() => { menuOpen = false; renameTo = entry.name; renameOpen = true; }}><Icon name="edit" size={20} /> Rename</button>
        <button onclick={() => { menuOpen = false; queryText = entry.smart ?? ''; queryOpen = true; }}><Icon name="search" size={20} /> Change the search</button>
        <button class="danger" onclick={() => { menuOpen = false; deleteOpen = true; }}><Icon name="trash" size={20} /> Delete smart album</button>
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

  <Sheet open={deleteOpen} title="Delete “{entry.name}”?" onclose={() => (deleteOpen = false)} width={380}>
    <div class="stack">
      {#if smart}
        <p class="muted">The smart album goes from all your devices. No photos are deleted.</p>
      {:else}
        <p class="muted">The album goes from all your devices. Its {plural(entry.count, 'photo')} stay in your library.</p>
        {#if entry.sharedId}<p class="muted">The shared album stays for the people in it; it just stops getting new photos from this album.</p>{/if}
        <p class="muted small">Any web links to this album stop working.</p>
      {/if}
      <Button kind="danger" block disabled={deleting} onclick={remove}>{deleting ? 'Deleting…' : 'Delete album'}</Button>
      <Button kind="ghost" block onclick={() => (deleteOpen = false)}>Cancel</Button>
    </div>
  </Sheet>

  <Sheet open={noteOpen} title="Description" onclose={() => (noteOpen = false)} width={420}>
    <form class="stack" onsubmit={(e) => { e.preventDefault(); saveNote(); }}>
      <textarea class="input area" rows="4" maxlength="1000" placeholder="What is this album about?" aria-label="Description" bind:value={noteText}></textarea>
      <Button type="submit" kind="primary" block>Save</Button>
      {#if entry.note}<Button kind="ghost" block onclick={() => { noteText = ''; saveNote(); }}>Remove the description</Button>{/if}
    </form>
  </Sheet>

  <Sheet open={queryOpen} title="Change the search" onclose={() => (queryOpen = false)} width={400}>
    <form class="stack" onsubmit={(e) => { e.preventDefault(); saveQuery(); }}>
      <input class="input" bind:value={queryText} aria-label="Search" placeholder="Search for…" />
      <Button type="submit" kind="primary" block disabled={!queryText.trim()}>Save</Button>
    </form>
  </Sheet>

  <Sheet open={exportOpen} title={isHandheld() ? 'Save or send all photos' : 'Export “' + entry.name + '”'} onclose={() => (exportOpen = false)} width={420}>
    <div class="stack">
      {#if isHandheld()}
        <p class="muted">Opens the share sheet with the album's {plural(entry.count, 'photo')} at full size: choose Save to Files or another app. Originals not on this phone are fetched from your other devices first.</p>
        <Button kind="primary" block disabled={exporting} onclick={shareAll}>{exporting ? 'Getting the photos ready…' : 'Continue'}</Button>
      {:else}
        <p class="muted">Copies the originals into a folder named after the album, inside the folder you choose. Your library does not change; exporting again only adds what is new.</p>
        {#if canPick()}
          <Button kind="primary" block disabled={exporting} onclick={chooseAndExport}>{exporting ? 'Exporting…' : 'Choose a folder…'}</Button>
        {:else}
          <form class="stack" onsubmit={(e) => { e.preventDefault(); exportTo(exportDir); }}>
            <input class="input" bind:value={exportDir} aria-label="Folder" placeholder="/path/to/folder" />
            <Button type="submit" kind="primary" block disabled={exporting || !exportDir.trim()}>{exporting ? 'Exporting…' : 'Export'}</Button>
          </form>
        {/if}
      {/if}
    </div>
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
  /* Basis auto: the buttons wrap to their own line before the name is cut ("The bo…" on the S25). */
  .who { min-width: 0; flex: 1 1 auto; display: grid; }
  h2 { margin: 0; font-size: var(--t-xl); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .acts { display: flex; align-items: center; gap: 8px; }
  .more { width: 40px; height: 40px; border: 0; border-radius: 50%; background: var(--surface-2); color: var(--ink); display: grid; place-items: center; cursor: pointer; }
  .note { flex: none; margin: -4px 18px 8px; padding: 0; border: 0; background: none; font: inherit; color: var(--ink-2, var(--ink)); text-align: left; white-space: pre-wrap; cursor: pointer; max-width: 70ch; }
  .area { height: auto; padding: 10px 12px; resize: vertical; }
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
