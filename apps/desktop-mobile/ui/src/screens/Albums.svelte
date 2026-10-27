<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api, blobUrl, type AlbumInfo } from '../lib/api';
  import { plural } from '../lib/format';
  import { app, fail } from '../lib/store.svelte';
  import { go } from '../lib/route.svelte';
  import Button from '../components/Button.svelte';
  import { pullRefresh } from '../lib/gestures';
  import { refreshEverything } from '../lib/refresh';

  let albums = $state<AlbumInfo[]>([]);
  let name = $state('');
  async function load() { albums = await api.albums(); }
  onMount(() => { load().catch(fail); });
  $effect(() => { app.version; untrack(() => { load().catch(() => {}); }); });

  async function create() {
    if (!name.trim()) return;
    try {
      const a = await api.createAlbum(name);
      name = '';
      go(`album/${a.id}`);
    } catch (e) { fail(e); }
  }
</script>

<div class="page" use:pullRefresh={{ onRefresh: refreshEverything }}>
  <form class="row-flex" onsubmit={(e) => { e.preventDefault(); create(); }}>
    <input class="input grow" placeholder="New album name" bind:value={name} />
    <Button type="submit" kind="primary" disabled={!name.trim()}>Create album</Button>
  </form>
  {#if albums.length === 0}
    <div style="margin-top:24px"><p class="muted">No albums yet. Name one above, then select photos in the library and choose “Add to album”.</p><Button onclick={() => go('library')}>Open the library</Button></div>
  {:else}
    <div class="grid">
      {#each albums as a (a.id)}
        <a class="album" href="#/album/{a.id}">
          <div class="cover">{#if a.cover}<img src={blobUrl(a.cover, 'thumb')} alt="" loading="lazy" />{:else}<span>▤</span>{/if}</div>
          <b>{a.name}{#if a.shared_as}<span class="pill">Shared</span>{/if}</b>
          <span class="muted small">{plural(a.item_count, 'photo')}</span>
        </a>
      {/each}
    </div>
  {/if}
</div>

<style>
  .page { padding: 18px; overflow: auto; height: 100%; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 16px; margin-top: 18px; }
  .album { text-decoration: none; display: flex; flex-direction: column; gap: 2px; }
  .cover { aspect-ratio: 1; border-radius: 12px; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; font-size: 34px; color: var(--ink-3); margin-bottom: 6px; }
  .cover img { width: 100%; height: 100%; object-fit: cover; }
  .pill { margin-left: 6px; font-size: 11px; font-weight: 600; padding: 1px 7px; border-radius: 999px; background: var(--accent-soft); color: var(--accent); vertical-align: 2px; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
