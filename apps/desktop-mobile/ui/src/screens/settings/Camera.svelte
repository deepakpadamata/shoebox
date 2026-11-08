<script lang="ts">
  // The camera backup page (U1-14): the switch with its rules and folders, and what this device
  // keeps in full. The switch alone is on the Settings home.
  import { onMount } from 'svelte';
  import { viewport } from '../../lib/viewport.svelte';
  import { api } from '../../lib/api';
  import BackupCard from '../../components/BackupCard.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import { cfg, loadSettings, save } from './store.svelte';
  onMount(() => { loadSettings(); });
  const s = $derived(cfg.s);
</script>

<BackupCard />

{#if s}
{#if s.role === 'companion'}
  <div class="set-section">{viewport.mobile ? 'Photos on this phone' : 'What this device keeps'}</div>
  <div class="card pad stack">
    <p class="small muted" style="margin:0">Keeps the full-size photos from this recent period, from any of your devices. Older ones stay on your keepers; open one and it is fetched. Pinned photos are always kept.</p>
    <Segmented value={String(s.keep_recent_days)} options={[{ value: '0', label: 'Off' }, { value: '30', label: '30 days' }, { value: '90', label: '90 days' }, { value: '365', label: '1 year' }]}
      onchange={(v) => save(() => api.setKeepRules(Number(v), s.keep_favorites), 'Saved')} />
    <label class="row-flex"><Toggle checked={s.keep_favorites} label="Keep favourites" onchange={(v) => save(() => api.setKeepRules(s.keep_recent_days, v), 'Saved')} /> Also keep my favourites</label>
  </div>
{/if}


{/if}

<style>
  .pad { padding: 14px; }
</style>
