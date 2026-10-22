<script lang="ts">
  // Settings (mockup 6b/6c): this device, how many copies to keep, relay, watched folders, theme.
  import { onMount } from 'svelte';
  import { api, type Settings } from '../lib/api';
  import { app, fail, toast } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';
  import List from '../components/List.svelte';
  import Row from '../components/Row.svelte';
  import Segmented from '../components/Segmented.svelte';

  let s = $state<Settings | null>(null);
  let name = $state('');
  let relay = $state('default');
  let customRelay = $state('');
  let folder = $state('');
  let target = $state('2');
  let theme = $state(localStorage.getItem('theme') ?? 'system');
  let exportDir = $state('');
  let exporting = $state(false);

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

  async function load() {
    s = await api.settings();
    name = s.device_name;
    target = String(s.redundancy_target);
    relay = s.relay === 'default' || s.relay === 'off' ? s.relay : 'custom';
    customRelay = relay === 'custom' ? s.relay : '';
  }
  onMount(() => { load().catch(fail); });

  async function save<T>(f: () => Promise<T>, msg: string) {
    try {
      await f();
      toast(msg);
      await load();
    } catch (e) {
      fail(e);
    }
  }

  function applyTheme(v: string) {
    theme = v;
    localStorage.setItem('theme', v);
    if (v === 'system') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', v);
  }
</script>

<div class="page">
  {#if s}
    <div class="section">This device</div>
    <div class="card pad stack">
      <span class="small muted">Name</span>
      <div class="row-flex">
        <input class="input grow" bind:value={name} />
        <Button size="sm" disabled={!name.trim() || name === s.device_name} onclick={() => save(() => api.renameDevice(name), 'Renamed')}>Save</Button>
      </div>
      <div class="small muted">Role: <b>{s.role}</b></div>
    </div>

    <div class="section">Copies to keep</div>
    <div class="card pad stack">
      <p class="small muted" style="margin:0">Photos with fewer copies than this are marked at risk and copied to other devices as soon as one is reachable.</p>
      <Segmented bind:value={target} options={['1', '2', '3', '4'].map((v) => ({ value: v, label: `${v} ${v === '1' ? 'copy' : 'copies'}` }))}
        onchange={(v) => save(() => api.setRedundancyTarget(Number(v)), 'Saved')} />
    </div>

    <div class="section">Connection</div>
    <div class="card pad stack">
      <p class="small muted" style="margin:0">Devices find each other on your network first. A relay only helps when they cannot reach each other directly; it never sees your photos. Changes apply the next time the app starts.</p>
      <Segmented bind:value={relay} options={[{ value: 'default', label: 'Default relays' }, { value: 'custom', label: 'My own relay' }, { value: 'off', label: 'No relay' }]}
        onchange={(v) => v !== 'custom' && save(() => api.setRelay(v), 'Saved. Restart to apply.')} />
      {#if relay === 'custom'}
        <div class="row-flex">
          <input class="input grow" placeholder="https://relay.example.com" bind:value={customRelay} />
          <Button size="sm" disabled={!customRelay.trim()} onclick={() => save(() => api.setRelay(customRelay), 'Saved. Restart to apply.')}>Save</Button>
        </div>
      {/if}
    </div>

    <div class="section">Watched folders</div>
    <List>
      {#each s.watched_folders as f}<Row icon="📁" title={f} />{/each}
      {#if s.watched_folders.length === 0}<Row title="No folders watched" subtitle="Photos that appear in a watched folder are imported automatically." />{/if}
    </List>
    <div class="row-flex" style="margin-top:8px">
      <input class="input grow" placeholder="/Users/you/Pictures/Inbox" bind:value={folder} />
      <Button size="sm" disabled={!folder.trim()} onclick={() => save(async () => { await api.watchFolder(folder); folder = ''; }, 'Watching')}>Watch</Button>
    </div>

    <div class="section">Export</div>
    <div class="card pad stack">
      <p class="small muted" style="margin:0">Copies every original on this device into plain folders by year and month, with a <span class="mono">library.json</span> listing favorites and albums. Your library is never locked in.</p>
      <div class="row-flex">
        <input class="input grow" placeholder="/Users/you/Shoebox export" bind:value={exportDir} />
        <Button size="sm" disabled={!exportDir.trim() || exporting} onclick={doExport}>{exporting ? 'Exporting…' : 'Export'}</Button>
      </div>
    </div>

    <div class="section">Appearance</div>
    <div class="card pad">
      <Segmented value={theme} options={[{ value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }]} onchange={applyTheme} />
    </div>

    <div class="section">Recovery</div>
    <div class="card pad small">
      {#if app.state?.recovery_kit_saved}✓ You confirmed the recovery kit is saved.{:else}The recovery kit has not been confirmed as saved.{/if}
    </div>
  {:else}
    <span class="muted">Loading…</span>
  {/if}
</div>

<style>
  .page { padding: 18px; max-width: 640px; overflow: auto; height: 100%; }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .section:first-child { margin-top: 0; }
  .pad { padding: 14px; }
  .input { height: 36px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
