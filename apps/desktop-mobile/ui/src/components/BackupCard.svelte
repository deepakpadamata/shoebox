<script lang="ts">
  // "Back up camera roll" (mockups 1d/5e): the switch, the rules, and what it is doing now.
  import { api, type BackupPolicy } from '../lib/api';
  import { app, fail, toast } from '../lib/store.svelte';
  import { plural } from '../lib/format';
  import Toggle from './Toggle.svelte';
  import ProgressBar from './ProgressBar.svelte';

  const st = $derived(app.backup);
  let busy = $state(false);

  async function apply(patch: Partial<BackupPolicy>) {
    if (!st) return;
    busy = true;
    try {
      app.backup = await api.backupConfigure({ ...st.policy, ...patch });
      if (patch.enabled !== false) {
        const r = await api.backupRun();
        app.backup = await api.backupStatus();
        if (r.imported > 0) toast(`Backed up ${plural(r.imported, 'photo')}`);
        else if (r.failed.length) toast(r.failed[0]);
      }
    } catch (e) {
      fail(e);
      app.backup = await api.backupStatus().catch(() => app.backup);
    } finally {
      busy = false;
    }
  }
</script>

{#if st && st.permission !== 'unavailable'}
  <div class="card pad stack">
    <div class="row-flex">
      <div class="grow">
        <b>Back up camera roll</b>
        <div class="small muted">New photos and videos are copied into your library, then to your other devices.</div>
      </div>
      <Toggle checked={st.policy.enabled} label="Back up camera roll" onchange={(v) => apply({ enabled: v })} />
    </div>

    {#if st.policy.enabled}
      {#if st.paused}
        <div class="small paused">⏸ {st.paused}. {plural(st.pending, 'photo')} waiting.</div>
      {:else if app.backupProgress}
        <ProgressBar total={app.backupProgress.total} segments={[{ value: app.backupProgress.done, tone: 'info' }]} label="Backing up" />
        <div class="small muted">{app.backupProgress.done} of {app.backupProgress.total}</div>
      {:else if st.pending > 0}
        <div class="small muted">{plural(st.pending, 'photo')} waiting.</div>
      {:else}
        <div class="small muted">✓ {plural(st.imported, 'photo')} backed up from this device.</div>
      {/if}
      {#if st.permission === 'limited'}
        <div class="small paused">Only the photos you chose are shared with the app. Allow all photos in the system settings to back up everything.</div>
      {/if}
      <label class="row-flex small"><Toggle checked={st.policy.wifi_only} label="Wi-Fi only" onchange={(v) => apply({ wifi_only: v })} /> Only on Wi-Fi</label>
      <label class="row-flex small"><Toggle checked={st.policy.charging_only} label="Only while charging" onchange={(v) => apply({ charging_only: v })} /> Only while charging</label>
      {#if st.last_error}<div class="small err">Last problem: {st.last_error}</div>{/if}
    {:else if st.permission === 'denied'}
      <div class="small muted">Photo access has not been allowed yet; you will be asked when you turn this on.</div>
    {/if}
  </div>
{/if}

<style>
  .pad { padding: 14px; }
  .paused { color: var(--warn); }
  .err { color: var(--risk); }
  label { cursor: pointer; }
</style>
