<script lang="ts">
  // Backup (U1-12): one answer first (the sentence from the core's backup status), then the
  // devices that hold the library, then the details (copies chart, queue, Free up space).
  // Storage and Devices used to be two screens that answered parts of the same question.
  import { go, param } from '../lib/route.svelte';
  import { app } from '../lib/store.svelte';
  import { safetyLine } from '../lib/safety';
  import { isHandheld } from '../lib/pick';
  import { viewport } from '../lib/viewport.svelte';
  import AddDeviceSheet from '../components/AddDeviceSheet.svelte';
  import BackupCard from '../components/BackupCard.svelte';
  import Button from '../components/Button.svelte';
  import Icon, { type IconName } from '../components/Icon.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Devices from './Devices.svelte';
  import Details from './backup/Details.svelte';

  const sub = $derived(param() ?? '');
  const line = $derived(safetyLine(app.backup_status, isHandheld() ? 'phone' : 'computer'));
  const icon = $derived<IconName>(({ ok: 'check', neutral: 'info', info: 'sync', warn: 'pause', risk: 'warning' } as const)[line.tone]);
  let addOpen = $state(false);
  const tabs = [{ value: '', label: 'Overview' }, { value: 'devices', label: 'Devices' }, { value: 'details', label: 'Storage' }];
</script>

<div class="page">
  {#if !viewport.mobile}
    <div class="tabs"><Segmented value={sub} options={tabs} onchange={(v) => go(v ? `backup/${v}` : 'backup')} /></div>
  {/if}

  {#snippet hero()}
    <section class="hero {line.tone}" aria-live="polite">
      <span class="badge"><Icon name={icon} size={26} /></span>
      <div class="txt">
        <h2>{line.short}</h2>
        <p>{line.long}</p>
        {#if app.backup_status?.state === 'busy' && app.backupProgress}
          <ProgressBar total={app.backupProgress.total} segments={[{ value: app.backupProgress.done, tone: 'info' }]} label="Backing up" />
        {/if}
        {#if app.backup_status?.state === 'single_device'}
          <div class="act"><Button kind="primary" onclick={() => (addOpen = true)}><Icon name="add" size={16} /> Add a device</Button></div>
        {:else if !viewport.mobile && (app.backup_status?.state === 'waiting' || app.backup_status?.state === 'at_risk')}
          <div class="act"><Button onclick={() => go('backup/devices')}>Show devices</Button></div>
        {/if}
      </div>
    </section>
  {/snippet}

  {#if viewport.mobile}
    <!-- One screen on a phone: the answer, the camera rules, the devices, then what is stored. -->
    {@render hero()}
    {#if app.backup && app.backup.permission !== 'unavailable'}<BackupCard />{/if}
    <div class="section">Devices</div>
    <Devices mode="devices" />
    <div class="section">Storage and copies</div>
    <Details />
    <div class="gap"><Devices mode="activity" /></div>
  {:else if sub === 'devices'}
    {@render hero()}
    <Devices mode="devices" />
  {:else if sub === 'details'}
    {@render hero()}
    <Details />
    <div class="gap"></div>
    <Devices mode="activity" />
  {:else}
    {@render hero()}
    {#if app.backup && app.backup.permission !== 'unavailable'}<div class="gap"><BackupCard /></div>{/if}
    <div class="gap"><Devices mode="activity" /></div>
  {/if}
</div>

<AddDeviceSheet open={addOpen} onclose={() => (addOpen = false)} />

<style>
  .page { padding: 18px; max-width: 1080px; height: 100%; overflow: auto; }
  @media (max-width: 720px) { .page { padding: 12px 14px calc(16px + var(--safe-bottom)); } }
  .tabs { max-width: 420px; margin-bottom: 14px; }
  .gap { margin-top: 14px; }
  .hero { display: flex; gap: 14px; align-items: flex-start; padding: 16px; border-radius: var(--radius); background: var(--bg-c); color: var(--ink); margin-bottom: 14px; }
  .badge { width: 44px; height: 44px; border-radius: 50%; display: grid; place-items: center; flex: none; background: var(--c); color: var(--surface); }
  .txt { min-width: 0; flex: 1; display: grid; gap: 6px; }
  h2 { margin: 0; font-size: var(--t-xl); letter-spacing: -.2px; }
  p { margin: 0; color: var(--ink-2); font-size: var(--t-md); }
  .act { margin-top: 6px; }
  .ok { --c: var(--ok); --bg-c: var(--ok-soft); }
  .neutral { --c: var(--ink-2); --bg-c: var(--surface-2); }
  .info { --c: var(--info); --bg-c: var(--info-soft); }
  .warn { --c: var(--warn); --bg-c: var(--warn-soft); }
  .risk { --c: var(--risk); --bg-c: var(--risk-soft); }
  .section { font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 18px 0 4px; }
</style>
