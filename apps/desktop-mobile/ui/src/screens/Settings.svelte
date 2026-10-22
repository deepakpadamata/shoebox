<script lang="ts">
  // Settings (mockup 6b/6c): this device, how many copies to keep, relay, watched folders, theme.
  import { onMount } from 'svelte';
  import { api, type Settings } from '../lib/api';
  import { app, fail, refreshState, toast } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';
  import List from '../components/List.svelte';
  import Row from '../components/Row.svelte';
  import Segmented from '../components/Segmented.svelte';
  import BackupCard from '../components/BackupCard.svelte';
  import Sheet from '../components/Sheet.svelte';
  import { canPick, pickFolder } from '../lib/pick';
  import { applyTextSize, savedTextSize } from '../lib/textsize';

  let s = $state<Settings | null>(null);
  let name = $state('');
  let relay = $state('default');
  let customRelay = $state('');
  let folder = $state('');
  let target = $state('2');
  let theme = $state(localStorage.getItem('theme') ?? 'system');
  let exportDir = $state('');
  let exporting = $state(false);
  let textSize = $state(savedTextSize());
  function setTextSize(v: string) {
    textSize = v;
    try { localStorage.setItem('textSize', v); } catch { /* not remembered */ }
    applyTextSize(v);
  }
  let diag = $state<{ file_name: string; text: string } | null>(null);

  async function showDiagnostics() {
    try { diag = await api.diagnostics(); } catch (e) { fail(e); }
  }
  async function copyDiagnostics() {
    try { await navigator.clipboard.writeText(diag!.text); toast('Copied. Paste it into your message.'); } catch { toast('Select the text and copy it'); }
  }
  const diagUrl = $derived(diag ? URL.createObjectURL(new Blob([diag.text], { type: 'text/plain' })) : '');

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
    <BackupCard />

    <div class="section">This device</div>
    <div class="card pad stack">
      <span class="small muted">Name</span>
      <div class="row-flex">
        <input class="input grow" bind:value={name} />
        <Button size="sm" disabled={!name.trim() || name === s.device_name} onclick={() => save(async () => { await api.renameDevice(name); await refreshState(); }, 'Renamed')}>Save</Button>
      </div>
      <div class="small muted">Role: <b>{s.role}</b></div>
    </div>

    <div class="section">Copies to keep</div>
    <div class="card pad stack">
      <p class="small muted" style="margin:0">Photos with fewer copies than this are marked at risk and copied to other devices as soon as one is reachable. A photo held by a keeper counts as backed up whatever this number is.</p>
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
      {#if canPick()}
        <Button size="sm" onclick={async () => { const d = await pickFolder('Watch a folder').catch(() => null); if (d) save(async () => { await api.watchFolder(d); }, 'Watching'); }}>Choose a folder to watch…</Button>
      {:else}
      <input class="input grow" placeholder="/Users/you/Pictures/Inbox" bind:value={folder} />
      <Button size="sm" disabled={!folder.trim()} onclick={() => save(async () => { await api.watchFolder(folder); folder = ''; }, 'Watching')}>Watch</Button>
      {/if}
    </div>

    <div class="section">Export</div>
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

    <div class="section">Help</div>
    <div class="card pad stack">
      <p class="small muted" style="margin:0">If something is wrong, you can send a report to whoever is helping you. It has counts, versions and recent errors, and nothing about what is in your library: no photos, names, places or file names. Nothing is sent automatically.</p>
      <div><Button size="sm" onclick={showDiagnostics}>Create a diagnostics report</Button></div>
    </div>

    <div class="section">Appearance</div>
    <div class="card pad">
      <Segmented value={theme} options={[{ value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }]} onchange={applyTheme} />
      <div class="small muted" style="margin:12px 0 6px">Text size</div>
      <Segmented value={textSize} options={[{ value: 'small', label: 'Small' }, { value: 'default', label: 'Default' }, { value: 'large', label: 'Large' }, { value: 'xlarge', label: 'Largest' }]} onchange={setTextSize} />
    </div>

    <div class="section">Recovery</div>
    <div class="card pad small stack">
      {#if app.state?.recovery_kit_saved}✓ You confirmed the recovery kit is saved.{:else}The recovery kit has not been confirmed as saved.{/if}
      <div class="muted">If every other device is lost, a new device recovers the library with the 24 words plus this device's link.</div>
      <div><Button size="sm" onclick={async () => { try { const l = await api.deviceLink(); await navigator.clipboard.writeText(l); toast('This device’s link was copied'); } catch (e) { fail(e); } }}>Copy this device’s link</Button></div>
    </div>
  {:else}
    <span class="muted">Loading…</span>
  {/if}
</div>

<Sheet open={diag !== null} title="Diagnostics report" onclose={() => (diag = null)} width={640}>
  {#if diag}
    <div class="stack">
      <p class="small muted" style="margin:0">Read it first: this is everything that would be shared.</p>
      <textarea class="report mono" readonly rows="14" value={diag.text}></textarea>
      <div class="row-flex">
        <Button kind="primary" onclick={copyDiagnostics}>Copy</Button>
        <a class="dl" href={diagUrl} download={diag.file_name}>Save as a file</a>
      </div>
    </div>
  {/if}
</Sheet>

<style>
  .report { width: 100%; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); font-size: 11px; padding: 10px; }
  .dl { font-size: 13px; color: var(--accent); }
  .page { padding: 18px; max-width: 640px; overflow: auto; height: 100%; }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .section:first-child { margin-top: 0; }
  .pad { padding: 14px; }
  .input { height: 36px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
