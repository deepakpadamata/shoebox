<script lang="ts">
  // Watched folders and Export: computers only (a phone has no folder to type or pick).
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { fail, toast } from '../../lib/store.svelte';
  import { canPick, pickFolder } from '../../lib/pick';
  import Button from '../../components/Button.svelte';
  import List from '../../components/List.svelte';
  import Row from '../../components/Row.svelte';
  import { cfg, loadSettings, save } from './store.svelte';

  let folder = $state('');
  let exportDir = $state('');
  let exporting = $state(false);
  onMount(() => { loadSettings(); });
  const s = $derived(cfg.s);
  async function doExport() {
    exporting = true;
    try {
      const r = await api.exportLibrary(exportDir.trim());
      toast(`Exported ${r.written} originals${r.missing ? `; ${r.missing} are on other devices and were skipped` : ''}`);
    } catch (e) {
      fail(e);
    } finally {
      exporting = false;
    }
  }
</script>

{#if s}
<div class="set-section">Watched folders</div>
<List>
  {#each s.watched_folders as f}<Row icon="folder" title={f} />{/each}
  {#if s.watched_folders.length === 0}<Row title="No folders watched" subtitle="Photos that appear in a watched folder are imported automatically." />{/if}
</List>
<div class="row-flex" style="margin-top:8px">
  {#if canPick()}
    <Button size="sm" onclick={async () => { const d = await pickFolder('Watch a folder').catch(() => null); if (d) save(async () => { await api.watchFolder(d); }, 'Watching'); }}>Choose a folder to watch…</Button>
  {:else}
  <input class="input grow" placeholder="/Users/you/Pictures/Inbox" bind:value={folder} />
  <Button size="sm" disabled={!folder.trim()} onclick={() => save(async () => { await api.watchFolder(folder); folder = ''; }, 'Watching')}>Watch</Button>
  {/if}
</div>

<div class="set-section">Export</div>
<div class="card pad stack">
  <p class="small muted" style="margin:0">Copies every original on this device into plain folders by year and month, with a <span class="mono">library.json</span> listing favorites and albums. Your library is never locked in.</p>
  <div class="row-flex">
    {#if canPick()}
      <Button size="sm" disabled={exporting} onclick={async () => { const d = await pickFolder('Export to a folder').catch(() => null); if (d) { exportDir = d; await doExport(); } }}>{exporting ? 'Exporting…' : 'Choose a folder and export…'}</Button>
    {:else}
    <input class="input grow" placeholder="/Users/you/Shoebox export" bind:value={exportDir} />
    <Button size="sm" disabled={!exportDir.trim() || exporting} onclick={doExport}>{exporting ? 'Exporting…' : 'Export'}</Button>
    {/if}
  </div>
</div>
{/if}

<style>
  .pad { padding: 14px; }
  .input { height: 36px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
