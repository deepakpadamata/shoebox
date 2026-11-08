<script lang="ts">
  // Advanced (U1-14): the rarely needed and the risky. Copies to keep, relays, rebuilding the
  // search index, diagnostics, watched folders and export (computers), and, in a card of their
  // own, replacing the recovery phrase and leaving the library.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { fail, refreshState, toast } from '../../lib/store.svelte';
  import { isHandheld } from '../../lib/pick';
  import Button from '../../components/Button.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import Sheet from '../../components/Sheet.svelte';
  import RecoveryKit from '../RecoveryKit.svelte';
  import Folders from './Folders.svelte';
  import { cfg, loadSettings, save } from './store.svelte';

  let target = $state('2');
  let relay = $state('default');
  let customRelay = $state('');
  let rebuilding = $state(false);
  onMount(() => {
    loadSettings().then(() => {
      const s = cfg.s;
      if (!s) return;
      target = String(s.redundancy_target);
      relay = s.relay === 'default' || s.relay === 'off' ? s.relay : 'custom';
      customRelay = relay === 'custom' ? s.relay : '';
    });
  });
  const s = $derived(cfg.s);

  let diag = $state<{ file_name: string; text: string } | null>(null);
  async function showDiagnostics() {
    try { diag = await api.diagnostics(); } catch (e) { fail(e); }
  }
  async function copyDiagnostics() {
    try { await navigator.clipboard.writeText(diag!.text); toast('Copied. Paste it into your message.'); } catch { toast('Select the text and copy it'); }
  }
  const diagUrl = $derived(diag ? URL.createObjectURL(new Blob([diag.text], { type: 'text/plain' })) : '');

  // Replacing the recovery phrase: the current one in, the new kit out (shown once).
  let replaceOpen = $state(false);
  let oldPhrase = $state('');
  let replacing = $state(false);
  let newKit = $state<{ phrase: string; kit_text: string } | null>(null);
  async function replacePhrase() {
    replacing = true;
    try {
      newKit = await api.replaceRecoveryPhrase(oldPhrase);
      oldPhrase = '';
      await refreshState();
    } catch (e) {
      fail(e);
    } finally {
      replacing = false;
    }
  }

  // Leaving the library: the data folder is set aside (not deleted) and the app restarts at first run.
  let leaveOpen = $state(false);
  let leaving = $state(false);
  async function leave() {
    leaving = true;
    try {
      await api.leaveLibrary();
      toast(isHandheld() ? 'Left the library. Shoebox closes; open it again to start over.' : 'Left the library. Shoebox is restarting…');
    } catch (e) {
      fail(e);
      leaving = false;
    }
  }
</script>

{#if s}
<div class="set-section">Copies to keep</div>
<div class="card pad stack">
  <p class="small muted" style="margin:0">Photos with fewer copies than this are marked at risk and copied to other devices as soon as one is reachable. A photo held by a keeper counts as backed up whatever this number is.</p>
  <Segmented bind:value={target} options={['1', '2', '3', '4'].map((v) => ({ value: v, label: `${v} ${v === '1' ? 'copy' : 'copies'}` }))}
    onchange={(v) => save(() => api.setRedundancyTarget(Number(v)), 'Saved')} />
</div>

<div class="set-section">Connection</div>
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

<div class="set-section">Search index</div>
<div class="card pad stack">
  <p class="small muted" style="margin:0">If search misses photos it should find, this device can build its search data again. Your photos are not touched.</p>
  <div><Button size="sm" disabled={rebuilding} onclick={async () => { rebuilding = true; try { await api.rebuildSearchIndex(); toast('Search index rebuilt'); } catch (e) { fail(e); } finally { rebuilding = false; } }}>Rebuild this device's search index</Button></div>
</div>

<div class="set-section">Help</div>
<div class="card pad stack">
  <p class="small muted" style="margin:0">If something is wrong, you can send a report to whoever is helping you. It has counts, versions and recent errors, and nothing about what is in your library: no photos, names, places or file names. Nothing is sent automatically.</p>
  <div><Button size="sm" onclick={showDiagnostics}>Create a diagnostics report</Button></div>
</div>

{#if !isHandheld()}<Folders />{/if}

<div class="set-section">Danger zone</div>
<div class="card danger pad small stack">
  <div class="muted">If someone else may have seen your recovery phrase, replace it: the old one stops working on every device.</div>
  <div><Button size="sm" onclick={() => { replaceOpen = true; newKit = null; oldPhrase = ''; }}>Replace recovery phrase…</Button></div>
  <hr />
  <div class="muted">To create a new library on this device or join another one. This device stops syncing with this library; its data here is set aside, not deleted.</div>
  <div><Button size="sm" kind="danger" onclick={() => (leaveOpen = true)}>Leave this library…</Button></div>
</div>
{:else}
  <span class="muted">Loading…</span>
{/if}

<Sheet open={replaceOpen} title={newKit ? 'Your new recovery kit' : 'Replace recovery phrase'} onclose={() => { if (!newKit) { replaceOpen = false; oldPhrase = ''; } }} width={560}>
  {#if newKit}
    <RecoveryKit phrase={newKit.phrase} kitText={newKit.kit_text} onsaved={() => { replaceOpen = false; newKit = null; }} />
  {:else}
    <div class="stack">
      <p class="small">A new 24-word phrase replaces the current one. Your devices switch to it as soon as they are in touch; the old phrase then opens nothing. Devices you removed stay removed.</p>
      <p class="small muted">If you see a device in Devices you do not know, remove it first. Then keep the new phrase somewhere only you can reach.</p>
      <textarea class="phrase" rows="3" placeholder="Current 24-word recovery phrase" autocomplete="off" spellcheck="false" bind:value={oldPhrase}></textarea>
      <Button kind="primary" block disabled={replacing || !oldPhrase.trim()} onclick={replacePhrase}>{replacing ? 'Replacing…' : 'Replace'}</Button>
      <Button block onclick={() => { replaceOpen = false; oldPhrase = ''; }}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<Sheet open={leaveOpen} title="Leave this library?" onclose={() => { if (!leaving) leaveOpen = false; }} width={460}>
  <div class="stack">
    <p class="small">This device stops syncing and Shoebox starts over at first run, where you can create a new library or join another one.</p>
    <p class="small muted">Nothing is deleted: this device's copy of the library is moved aside in its data folder. Originals only this device holds are no longer backed up anywhere; check Storage first. The other devices keep listing this one until an admin removes it.</p>
    <Button kind="danger" block disabled={leaving} onclick={leave}>{leaving ? 'Leaving…' : 'Leave this library'}</Button>
    <Button block disabled={leaving} onclick={() => (leaveOpen = false)}>Cancel</Button>
  </div>
</Sheet>

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
  .pad { padding: 14px; }
  .report { width: 100%; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); font-size: var(--t-xs); padding: 10px; }
  .dl { font-size: var(--t-sm); color: var(--accent); }
  .phrase { width: 100%; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px 12px; color: var(--ink); font: inherit; resize: none; }
  .danger { border-color: color-mix(in srgb, var(--risk) 35%, var(--line)); }
  hr { border: 0; border-top: 1px solid var(--line); width: 100%; margin: 4px 0; }
  .input { height: 36px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
