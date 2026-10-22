<script lang="ts">
  // Recently Deleted (M3-11): photos stay recoverable for 30 days.
  import { onMount, untrack } from 'svelte';
  import { api, type PhotoSummary } from '../lib/api';
  import { plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import Tile from '../components/Tile.svelte';
  import Button from '../components/Button.svelte';

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
</script>

<div class="page">
  {#if items.length === 0}
    <div class="empty"><b>Nothing here</b><span class="muted">Deleted photos stay for 30 days, then are removed from every device.</span></div>
  {:else}
    <div class="row-flex bar">
      <span class="grow muted small">{plural(items.length, 'photo')} · removed from every device after 30 days</span>
      <Button size="sm" onclick={restore}>{selected.length ? `Restore ${selected.length}` : 'Restore all'}</Button>
    </div>
    <div class="grid">
      {#each items as p (p.id)}
        <Tile id={p.id} size={140} selected={selected.includes(p.id)} selecting={selected.length > 0} hasThumb={p.has_thumb} onclick={() => toggle(p.id)} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .page { padding: 18px; overflow: auto; height: 100%; }
  .bar { margin-bottom: 14px; }
  .grid { display: flex; flex-wrap: wrap; gap: 2px; }
  .empty { height: 100%; display: grid; place-content: center; text-align: center; gap: 6px; }
</style>
