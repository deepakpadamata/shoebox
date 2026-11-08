<script lang="ts">
  // You and this device (U1-14): the name people you share with see, and this device's own name.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { refreshState } from '../../lib/store.svelte';
  import Button from '../../components/Button.svelte';
  import { cfg, loadSettings, save } from './store.svelte';

  let profileName = $state('');
  let profileSet = $state(false);
  let name = $state('');
  onMount(() => {
    api.profile().then((p) => { profileName = p.name; profileSet = p.is_set; }).catch(() => {});
    loadSettings().then(() => (name = cfg.s?.device_name ?? ''));
  });
  const s = $derived(cfg.s);
</script>

<div class="set-section">You</div>
<div class="card pad stack">
  <span class="small muted">Your name</span>
  <div class="row-flex">
    <input class="input grow" bind:value={profileName} placeholder="Your name" />
    <Button size="sm" disabled={!profileName.trim()} onclick={() => save(async () => { await api.setProfileName(profileName); profileSet = true; }, 'Saved on all your devices')}>Save</Button>
  </div>
  <div class="small muted">{profileSet ? 'People you share albums with see this name. It is the same on all your devices.' : 'People you share albums with see this device\'s name until you set one here, for all your devices.'}</div>
</div>

{#if s}
  <div class="set-section">This device</div>
  <div class="card pad stack">
    <span class="small muted">Name</span>
    <div class="row-flex">
      <input class="input grow" bind:value={name} />
      <Button size="sm" disabled={!name.trim() || name === s.device_name} onclick={() => save(async () => { await api.renameDevice(name); await refreshState(); }, 'Renamed')}>Save</Button>
    </div>
    <div class="small muted">Role: <b>{s.role}</b></div>
  </div>
{/if}

<style>
  .pad { padding: 14px; }
  .input { height: 36px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
