<script lang="ts">
  import Icon from './Icon.svelte';
  // "Back up camera roll" (mockups 1d/5e): the switch, the rules, and what it is doing now.
  import { api, type BackupFolder, type BackupPolicy, type BackupPreview, type BackupScope } from '../lib/api';
  import { app, fail, toast } from '../lib/store.svelte';
  import { bytes, plural } from '../lib/format';
  import Toggle from './Toggle.svelte';
  import ProgressBar from './ProgressBar.svelte';
  import Sheet from './Sheet.svelte';
  import Button from './Button.svelte';

  // `compact`: only the switch and one line of status, for the Settings home (the rules and
  // folders are on the camera backup page).
  let { compact = false }: { compact?: boolean } = $props();
  const st = $derived(app.backup);
  let busy = $state(false);
  // A Mac backs up the Photos app's library (P1-04): no folders, Wi-Fi or charger rules.
  const photosApp = $derived(st?.source === 'photos_library');

  // Turning the Photos library on first shows what it would take; access is asked for then.
  let preview = $state<BackupPreview | null>(null);

  async function toggle(on: boolean) {
    if (!on || !photosApp) return apply({ enabled: on });
    busy = true;
    try {
      preview = await api.backupPreview();
    } catch (e) {
      fail(e);
      app.backup = await api.backupStatus().catch(() => app.backup);
    } finally {
      busy = false;
    }
  }

  async function startPhotos() {
    preview = null;
    await apply({ enabled: true, wifi_only: false, charging_only: false, scope: { kind: 'all' } });
  }

  const scopeLabel = (s: BackupScope) =>
    s.kind === 'camera' ? 'Camera photos' : s.kind === 'all' ? 'Every folder' : plural(s.folders.length, 'chosen folder');

  // "Choose folders": every gallery folder with a switch; saved as camera, all, or the list.
  let folders = $state<BackupFolder[] | null>(null);
  let chosen = $state(new Set<string>());
  let everything = $state(false);

  async function openFolders() {
    try {
      const list = await api.backupFolders();
      chosen = new Set(list.filter((f) => f.included).map((f) => f.path));
      everything = st?.policy.scope.kind === 'all';
      folders = list;
    } catch (e) {
      fail(e);
    }
  }

  function pick(path: string, on: boolean) {
    const next = new Set(chosen);
    if (on) next.add(path); else next.delete(path);
    chosen = next;
  }

  async function saveFolders(scope?: BackupScope) {
    if (!folders) return;
    if (!scope) {
      const camera = folders.filter((f) => f.camera).map((f) => f.path);
      const sameAsCamera = camera.length === chosen.size && camera.every((p) => chosen.has(p));
      scope = everything ? { kind: 'all' } : sameAsCamera ? { kind: 'camera' } : { kind: 'folders', folders: [...chosen] };
    }
    folders = null;
    await apply({ scope });
  }

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
        {#if photosApp}
          <b>Back up the Photos library</b>
          <div class="small muted">Photos and videos in the Photos app are copied into your library, then to your other devices. Edits and deletions in Photos follow.</div>
        {:else}
          <b>Back up camera roll</b>
          <div class="small muted">New photos and videos are copied into your library, then to your other devices.</div>
        {/if}
      </div>
      <Toggle checked={st.policy.enabled} label={photosApp ? 'Back up the Photos library' : 'Back up camera roll'} onchange={toggle} />
    </div>

    {#if st.policy.enabled}
      {#if st.paused}
        <div class="small paused"><Icon name="pause" size={14} /> {st.paused}. {plural(st.pending, 'photo')} waiting.</div>
      {:else if app.backupProgress}
        <ProgressBar total={app.backupProgress.total} segments={[{ value: app.backupProgress.done, tone: 'info' }]} label="Backing up" />
        <div class="small muted">{app.backupProgress.done} of {app.backupProgress.total}</div>
      {:else if st.pending > 0}
        <div class="small muted">{plural(st.pending, 'photo')} waiting.</div>
      {:else}
        <div class="small muted"><Icon name="check" size={14} /> {plural(st.imported, 'photo')} backed up from this device.</div>
      {/if}
      {#if st.cloud_waiting > 0 && !compact}
        <div class="small muted"><Icon name="cloud" size={14} /> {plural(st.cloud_waiting, 'photo')} in iCloud only: downloaded while this Mac is on mains power.</div>
      {/if}
      {#if st.permission === 'limited' && !compact}
        <div class="small paused">Only the photos you chose are shared with the app. Allow all photos in the system settings to back up everything.</div>
      {/if}
      {#if !photosApp && !compact}
      <div class="row-flex small">
        <span class="grow">Backing up: <b>{scopeLabel(st.policy.scope)}</b></span>
        <Button size="sm" disabled={busy} onclick={openFolders}>Choose folders</Button>
      </div>
      <label class="row-flex small"><Toggle checked={st.policy.wifi_only} label="Wi-Fi only" onchange={(v) => apply({ wifi_only: v })} /> Only on Wi-Fi</label>
      <label class="row-flex small"><Toggle checked={st.policy.charging_only} label="Only while charging" onchange={(v) => apply({ charging_only: v })} /> Only while charging</label>
      {/if}
      {#if st.last_error && !compact}<div class="small err">Last problem: {st.last_error}</div>{/if}
    {:else if st.permission === 'denied'}
      <div class="small muted">Photo access has not been allowed yet; you will be asked when you turn this on.</div>
    {/if}
  </div>
{/if}

<Sheet open={preview !== null} title="Back up the Photos library" onclose={() => (preview = null)} width={420}>
  {#if preview}
    <div class="stack">
      <div>
        {plural(preview.photos, 'photo')} and {plural(preview.videos, 'video')}, {bytes(preview.bytes)}. They are copied into your
        library on this Mac, so it needs about that much free space, and then to your other devices.
      </div>
      {#if preview.cloud_only > 0}
        <div class="small muted">{plural(preview.cloud_only, 'original')} {preview.cloud_only === 1 ? 'is' : 'are'} in iCloud only. They are downloaded while this Mac is on mains power.</div>
      {/if}
      {#if preview.permission === 'limited'}
        <div class="small paused">Only the photos you chose are shared with Shoebox, so only those are backed up. To back up everything, allow all photos in System Settings → Privacy &amp; Security → Photos.</div>
      {/if}
      <Button kind="primary" block disabled={busy} onclick={startPhotos}>Back up</Button>
      <Button block onclick={() => (preview = null)}>Not now</Button>
    </div>
  {/if}
</Sheet>

<Sheet open={folders !== null} title="Folders to back up" onclose={() => (folders = null)} width={480}>
  {#if folders}
    <div class="stack">
      <label class="row-flex small"><Toggle bind:checked={everything} label="Every folder" /> <span class="grow">Every folder, and new ones as they appear</span></label>
      {#if folders.length === 0}
        <div class="small muted">No photos found on this device yet.</div>
      {/if}
      <div class="folders" class:off={everything}>
        {#each folders as f (f.path)}
          <label class="row-flex folder">
            <Toggle checked={everything || chosen.has(f.path)} label={f.name} onchange={(v) => pick(f.path, v)} />
            <span class="grow name">
              <span>{f.name}</span>{#if f.camera && f.name !== 'Camera'}<span class="tag small">Camera</span>{/if}
              <span class="small muted block">{f.location}</span>
            </span>
            <span class="small muted">{f.photos.toLocaleString()}</span>
          </label>
        {/each}
      </div>
      <div class="small muted">Turning a folder off keeps what it already backed up.</div>
      <Button kind="primary" block disabled={busy} onclick={() => saveFolders()}>Save</Button>
      <Button block disabled={busy} onclick={() => saveFolders({ kind: 'camera' })}>Camera photos only</Button>
    </div>
  {/if}
</Sheet>

<style>
  .pad { padding: 14px; }
  .paused { color: var(--warn); }
  .err { color: var(--risk); }
  label { cursor: pointer; }
  .folders { display: flex; flex-direction: column; gap: 10px; max-height: 50vh; overflow: auto; }
  .folders.off { opacity: .5; pointer-events: none; }
  .block { display: block; }
  .name { min-width: 0; overflow-wrap: anywhere; }
  .tag { margin-left: 6px; padding: 1px 6px; border-radius: 6px; background: var(--surface-2); color: var(--ink-2); }
</style>
