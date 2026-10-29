<script lang="ts">
  // Recently Deleted (M3-11): photos stay recoverable for 30 days, or are deleted for good at once.
  import { onMount, untrack } from 'svelte';
  import { api, type PhotoSummary } from '../lib/api';
  import { plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import Tile from '../components/Tile.svelte';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';
  import { go } from '../lib/route.svelte';

  let items = $state<PhotoSummary[]>([]);
  let selected = $state<string[]>([]);
  async function load() { items = await api.recentlyDeleted(); selected = selected.filter((id) => items.some((p) => p.id === id)); }
  onMount(() => { load().catch(fail); });
  $effect(() => { app.version; untrack(() => { load().catch(() => {}); }); });

  function toggle(id: string) { selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]; }
  async function restore() {
    const ids = selected.length ? selected : items.map((p) => p.id);
    try {
      await api.restore(ids);
      toast(`Restored ${plural(ids.length, 'photo')}`);
      selected = [];
    } catch (e) { fail(e); }
  }

  // "Delete now": the selection, or everything here; confirmed first since it cannot be undone.
  let confirming = $state<string[] | null>(null);
  let deleting = $state(false);
  async function deleteNow() {
    if (!confirming) return;
    deleting = true;
    try {
      const n = await api.deletePermanently(confirming);
      toast(`Deleted ${plural(n, 'photo')} for good`);
      selected = [];
      confirming = null;
      await load();
    } catch (e) { fail(e); } finally { deleting = false; }
  }
</script>

<div class="page">
  {#if items.length === 0}
    <div class="empty"><b>Nothing here</b><span class="muted">Deleted photos stay for 30 days, then are removed from every device. You can also delete them sooner from here.</span><div style="margin-top:10px"><Button onclick={() => go('library')}>Back to the library</Button></div></div>
  {:else}
    <div class="row-flex bar">
      <span class="grow muted small">{plural(items.length, 'photo')} · removed from every device after 30 days</span>
      <Button size="sm" onclick={restore}>{selected.length ? `Restore ${selected.length}` : 'Restore all'}</Button>
      <Button size="sm" kind="danger" onclick={() => (confirming = selected.length ? [...selected] : items.map((p) => p.id))}>{selected.length ? `Delete ${selected.length} now` : 'Delete all now'}</Button>
    </div>
    <div class="grid">
      {#each items as p (p.id)}
        <Tile id={p.id} size={140} selected={selected.includes(p.id)} selecting={selected.length > 0} hasThumb={p.has_thumb} onclick={() => toggle(p.id)} />
      {/each}
    </div>
  {/if}
</div>

<Sheet open={confirming !== null} title="Delete for good?" onclose={() => { if (!deleting) confirming = null; }} width={440}>
  {#if confirming}
    <div class="stack">
      <p class="small">{plural(confirming.length, 'photo')} will be removed from your library on every device, keepers included, without waiting 30 days. This cannot be undone.</p>
      <p class="small muted">Files still in a phone's gallery or a folder on a computer are not touched.</p>
      <Button kind="danger" block disabled={deleting} onclick={deleteNow}>{deleting ? 'Deleting…' : `Delete ${plural(confirming.length, 'photo')}`}</Button>
      <Button block disabled={deleting} onclick={() => (confirming = null)}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<style>
  .page { padding: 18px; overflow: auto; height: 100%; }
  .bar { margin-bottom: 14px; }
  .grid { display: flex; flex-wrap: wrap; gap: 2px; }
  .empty { height: 100%; display: grid; place-content: center; text-align: center; gap: 6px; }
</style>
