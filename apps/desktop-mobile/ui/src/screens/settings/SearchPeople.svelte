<script lang="ts">
  // Search and People (U1-14): what this device indexes, and whether it looks for faces.
  import { onMount } from 'svelte';
  import { api, type SearchStatus } from '../../lib/api';
  import { phoneAloneLine, processingHeadline, processingRows } from '../../lib/search';
  import { viewport } from '../../lib/viewport.svelte';
  import { fail, toast } from '../../lib/store.svelte';
  import Button from '../../components/Button.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import { cfg, loadSettings, save } from './store.svelte';

  let search = $state<SearchStatus | null>(null);
  let people = $state<boolean | null>(null);
  onMount(() => {
    loadSettings();
    api.people().then((p) => (people = p.on)).catch(() => {});
    const refresh = () => api.searchStatus().then((st) => (search = st)).catch(() => {});
    refresh();
    const timer = setInterval(refresh, 3000);
    return () => clearInterval(timer);
  });
  const s = $derived(cfg.s);
  const pct = (n: number, of: number) => (of === 0 ? 100 : Math.floor((n * 100) / of));
</script>

<div class="set-section">People</div>
<div class="card pad stack">
  <label class="row-flex"><Toggle checked={!!people} label="Find people in photos" onchange={async (v) => { try { await api.setPeople(v); people = v; toast(v ? 'People is on' : 'People is off'); } catch (e) { fail(e); } }} /> Find people in photos</label>
  <p class="small muted" style="margin:0">Faces are found and grouped on your own devices and never leave them.</p>
</div>

{#if search && s && s.role !== 'viewer'}
  <div class="set-section">Search</div>
  <div class="card pad stack">
      <label class="row-flex"><Toggle checked={search.index_here} label="Index photos on this device"
        onchange={(v) => save(async () => { await api.setSearchIndexing(v); search = await api.searchStatus(); }, v ? 'Indexing on' : 'Indexing off')} />
        {viewport.mobile ? 'Index photos while charging' : 'Index photos on power when idle'}</label>
      <p class="small muted" style="margin:0">Indexing lets your devices find photos by what is in them. It is shared out between the devices that do it; turned off here, the others take this device's share.</p>
      {#if search.meaning && search.index_here}
        <div class="row-flex">
          {#if search.run_active}
            <Button size="sm" kind="ghost" onclick={() => save(async () => { await api.stopIndexNow(); }, 'Stopping…')}>Stop indexing</Button>
          {:else}
            <Button size="sm" onclick={() => save(async () => { if (!(await api.indexNow())) toast('Indexing is already running'); search = await api.searchStatus(); }, 'Indexing started')}>Index now</Button>
          {/if}
          <span class="tiny muted grow">{search.run_active ? (viewport.mobile ? 'Runs until everything is indexed. Keep the phone plugged in and the app open.' : 'Runs until everything is indexed. Keep the app open.') : (viewport.mobile ? 'Starts now, whether or not the phone is charging, until everything is indexed.' : 'Starts now, whether or not the computer is idle or on power, until everything is indexed.')}</span>
        </div>
        {#if search.last_run && !search.run_active}<div class="small">{search.last_run}</div>{/if}
      {/if}
      {#if processingHeadline(search)}<div class="small"><b>{processingHeadline(search)}</b></div>{/if}
      {#each processingRows(search) as r (r.key)}
        <div class="proc">
          <div class="row-flex between"><span class="small">{r.label}</span>{#if r.state !== 'off'}<span class="small muted">{r.done.toLocaleString()} of {r.total.toLocaleString()}</span>{/if}</div>
          {#if r.state !== 'off'}<div class="pbar" role="progressbar" aria-label={r.label} aria-valuenow={pct(r.done, r.total)} aria-valuemin="0" aria-valuemax="100"><span class={r.state} style:width="{pct(r.done, r.total)}%"></span></div>{/if}
          <div class="tiny muted">{r.note}</div>
        </div>
      {/each}
      {#if !search.meaning}<div class="small muted">This device searches by words and labels: the model for searching by meaning is not installed.</div>{/if}
      {#if search.model_download}<div class="small muted">Downloading search models: {pct(search.model_download.done, search.model_download.total)}% of {(search.model_download.total / 1e6).toFixed(0)} MB ({search.model_download.name}). It happens once, on Wi-Fi.</div>{/if}
      {#if phoneAloneLine(search)}<div class="small muted">{phoneAloneLine(search)}</div>{/if}
      {#each search.indexers as i}
        <div class="small muted">{i.this_device ? 'This device' : i.name || 'A device'}: {i.photos_per_min.toLocaleString()} photos a minute{i.reads_text ? ', reads text' : ''}{i.finds_faces ? ', finds faces' : ''}</div>
      {/each}
      <div class="small muted">Search data on this device: {(search.held_bytes / 1e6).toFixed(1)} MB</div>
  </div>
{:else if !search}
  <span class="muted">Loading…</span>
{/if}

<style>
  .proc { display: flex; flex-direction: column; gap: 4px; }
  .pbar { height: 6px; border-radius: 3px; background: var(--surface-2); overflow: hidden; }
  .pbar span { display: block; height: 100%; background: var(--ink-3); }
  .pbar span.working { background: var(--accent); }
  .pbar span.done { background: var(--ok, #3ba55d); }
  .pad { padding: 14px; }
  .tiny { font-size: var(--t-xs); }
</style>
