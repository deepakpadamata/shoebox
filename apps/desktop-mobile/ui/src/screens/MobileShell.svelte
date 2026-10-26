<script lang="ts">
  // The phone layout (mockups 2a/2d/5a): a large title, tab bar (Library · Albums · Shared ·
  // Devices), and Settings from the avatar. Screens are the same ones the desktop uses.
  import { api } from '../lib/api';
  import { go, param, section } from '../lib/route.svelte';
  import { app, fail, refreshState, toast } from '../lib/store.svelte';
  import { plural } from '../lib/format';
  import Avatar from '../components/Avatar.svelte';
  import Banner from '../components/Banner.svelte';
  import Sheet from '../components/Sheet.svelte';
  import Toggle from '../components/Toggle.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import { etaSeconds } from '../lib/store.svelte';
  import { hasKeepAwake, keepAwake } from '../lib/host';
  import Library from './Library.svelte';
  import Albums from './Albums.svelte';
  import Shared from './Shared.svelte';
  import Deleted from './Deleted.svelte';
  import Devices from './Devices.svelte';
  import Health from './Health.svelte';
  import Settings from './Settings.svelte';
  import RecoveryKit from './RecoveryKit.svelte';

  const tabs = [
    { id: 'library', label: 'Library', icon: '▦' },
    { id: 'albums', label: 'Albums', icon: '▤' },
    { id: 'shared', label: 'Shared', icon: '👥' },
    { id: 'devices', label: 'Devices', icon: '⇄' },
  ];
  const sec = $derived(section() || 'library');
  const tab = $derived(sec === 'album' ? 'albums' : ['favorites', 'deleted', 'health', 'settings'].includes(sec) ? '' : sec);
  const atRisk = $derived(app.counts?.at_risk ?? 0);
  let kitOpen = $state(false);
  let addOpen = $state(false);
  let adding = $state(false);
  // A phone with a camera roll can also add photos the backup does not see: other albums when
  // only some photos are shared with the app, Downloads, a cloud drive, a USB stick.
  const canAdd = $derived(!!app.backup && app.backup.permission !== 'unavailable' && app.state?.role !== 'viewer');
  async function answerHeld(del: boolean) {
    try {
      await api.answerGalleryDeletions(del);
      if (app.backup) app.backup = { ...app.backup, gallery_deletions_held: 0 };
      toast(del ? 'Deleted from your library (Recently Deleted for 30 days)' : 'Kept in your library');
    } catch (e) {
      fail(e);
    }
  }
  async function addPhotos(files: boolean) {
    adding = true;
    try {
      const r = await api.importPicked(files);
      addOpen = false;
      refreshState().catch(() => {});
      const n = r.imported + r.duplicates + r.skipped + r.failed.length;
      if (r.failed.length) fail(new Error(r.failed[0]));
      else if (n > 0) toast(`Added ${plural(r.imported, 'photo')}${r.duplicates ? `, ${r.duplicates} already in the library` : ''}${r.skipped ? `, ${plural(r.skipped, 'file')} not a photo or video` : ''}${r.failed.length ? `, ${r.failed.length} failed` : ''}`);
    } catch (e) {
      fail(e);
    } finally {
      adding = false;
    }
  }
  let menuOpen = $state(false);
  let awake = $state(false);
  const eta = $derived((app.counts, etaSeconds()));
  const etaText = $derived(eta === null ? '' : eta < 90 ? 'about a minute left' : eta < 3600 ? `about ${Math.ceil(eta / 60)} min left` : `about ${Math.ceil(eta / 3600)} h left`);
  const unreachable = $derived(atRisk > 0 && app.peersOnline === 0);
  const titles: Record<string, string> = { library: 'Library', favorites: 'Favorites', albums: 'Albums', album: 'Album', shared: 'Shared', deleted: 'Recently Deleted', devices: 'Devices', health: 'Storage', settings: 'Settings' };
</script>

<div class="phone">
  <header class="navbar">
    <div class="grow">
      {#if ['favorites', 'deleted', 'health', 'settings', 'album'].includes(sec)}
        <button class="back" onclick={() => go(sec === 'album' ? 'albums' : 'library')}>‹ Library</button>
      {/if}
      <h2>{titles[sec] ?? 'Library'}</h2>
      {#if sec === 'library'}
        <div class="small muted row-flex"><span class="sd" class:busy={app.syncing.length > 0}></span>{app.syncing.length > 0 ? 'Syncing…' : app.syncError ? 'Some devices are not reachable' : 'Up to date'}</div>
      {/if}
    </div>
    {#if canAdd && (sec === 'library' || sec === '')}
      <button class="add-btn" aria-label="Add photos" onclick={() => (addOpen = true)}>＋</button>
    {/if}
    <button class="avatar-btn" aria-label="Menu" onclick={() => (menuOpen = true)}>
      <Avatar name={app.state?.device_name ?? '?'} size={32} />
      {#if atRisk > 0}<span class="badge"></span>{/if}
    </button>
  </header>

  {#if (app.backup?.gallery_deletions_held ?? 0) > 0}
    <div class="banners">
      <Banner tone="warn" icon="🗑">
        <b>{plural(app.backup?.gallery_deletions_held ?? 0, 'photo')}</b> disappeared from this phone's gallery at once. Delete them from your library on every device too?
        <div class="row-flex held">
          <button class="choice small-choice" onclick={() => answerHeld(true)}>Delete them</button>
          <button class="choice small-choice" onclick={() => answerHeld(false)}>Keep them in the library</button>
        </div>
      </Banner>
    </div>
  {/if}

  {#if app.removed || app.state?.membership === 'removed'}
    <div class="banners"><Banner tone="risk" icon="⛔">This device was removed from the library.</Banner></div>
  {:else if unreachable && sec === 'library'}
    <div class="banners"><Banner tone="warn" icon="⏸" action="Devices" onaction={() => go('devices')}>Backup paused: none of your devices are reachable. Your photos stay safe on this device until one is.</Banner></div>
  {:else if atRisk > 0 && sec === 'library'}
    <div class="banners">
      <Banner tone="risk" action="View" onaction={() => go('health')}>
        <b>{plural(atRisk, 'photo')}</b> not backed up on enough devices yet.{etaText ? ` ${etaText}.` : ''}
        {#if app.backupProgress}<ProgressBar total={app.backupProgress.total} segments={[{ value: app.backupProgress.done, tone: 'info' }]} label="Backing up" />{/if}
        {#if hasKeepAwake()}<label class="row-flex small awake"><Toggle checked={awake} label="Keep the screen on" onchange={(v) => { awake = v; keepAwake(v); }} /> Keep the screen on until done</label>{/if}
      </Banner>
    </div>
  {:else if app.backup && app.backup.permission !== 'unavailable' && !app.backup.policy.enabled && sec === 'library'}
    <div class="banners"><Banner tone="info" icon="☁︎" action="Turn on" onaction={() => go('settings')}>Back up your camera roll to your other devices.</Banner></div>
  {:else if app.state && !app.state.recovery_kit_saved && sec === 'library'}
    <div class="banners"><Banner tone="accent" icon="🔑" action="I saved it" onaction={() => (kitOpen = true)}>Confirm you saved your recovery kit.</Banner></div>
  {/if}

  <main class="content">
    {#if sec === 'library' || sec === ''}<Library source="all" />
    {:else if sec === 'favorites'}<Library source="favorites" />
    {:else if sec === 'albums'}<Albums />
    {:else if sec === 'album'}{#key param()}<Library source="album" album={param()} />{/key}
    {:else if sec === 'shared'}<Shared />
    {:else if sec === 'deleted'}<Deleted />
    {:else if sec === 'devices'}<Devices />
    {:else if sec === 'health'}<Health />
    {:else if sec === 'settings'}<Settings />
    {:else}<Library source="all" />{/if}
  </main>

  <nav class="tabbar" aria-label="Sections">
    {#each tabs as t}
      <a class="t" class:on={tab === t.id} href="#/{t.id}"><span class="i">{t.icon}</span>{t.label}</a>
    {/each}
  </nav>
</div>

<Sheet open={menuOpen} title="This device" onclose={() => (menuOpen = false)}>
  <div class="menu">
    <a href="#/favorites" onclick={() => (menuOpen = false)}>♥ Favorites</a>
    <a href="#/deleted" onclick={() => (menuOpen = false)}>🗑 Recently Deleted</a>
    <a href="#/health" onclick={() => (menuOpen = false)}>◔ Storage{#if atRisk > 0} <span class="risk">{atRisk}</span>{/if}</a>
    <a href="#/settings" onclick={() => (menuOpen = false)}>⚙︎ Settings</a>
  </div>
</Sheet>

<Sheet open={addOpen} title="Add photos" onclose={() => { if (!adding) addOpen = false; }}>
  <div class="add">
    <p class="small muted">Photos from this phone's gallery stay where they are: Shoebox never keeps a second copy, and deleting one in the gallery deletes it here too.</p>
    <button class="choice" disabled={adding} onclick={() => addPhotos(false)}><b>{adding ? 'Adding…' : 'From the gallery'}</b><span class="small muted">{app.backup?.permission === 'granted' ? 'Any album on this phone' : 'Choose which photos Shoebox may see'}</span></button>
    <button class="choice" disabled={adding} onclick={() => addPhotos(true)}><b>From files</b><span class="small muted">A cloud drive, a USB stick… (copied in: the phone has no other copy)</span></button>
    {#if app.importProgress}<ProgressBar total={app.importProgress.total} segments={[{ value: app.importProgress.done, tone: 'info' }]} label="Adding" />{/if}
  </div>
</Sheet>

<Sheet open={kitOpen} title="Recovery kit" onclose={() => (kitOpen = false)}>
  <RecoveryKit onsaved={() => (kitOpen = false)} />
</Sheet>

<style>
  .phone { display: flex; flex-direction: column; height: 100%; background: var(--surface); padding-top: var(--safe-top); }
  .navbar { flex: none; display: flex; align-items: center; justify-content: space-between; padding: 6px 16px 10px; gap: 10px; }
  .navbar h2 { margin: 0; font-size: 28px; letter-spacing: -0.02em; }
  .back { background: none; border: 0; color: var(--accent); font-size: 15px; padding: 0; cursor: pointer; }
  .add-btn { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--accent-soft); color: var(--accent); font-size: 20px; line-height: 1; cursor: pointer; }
  .add { display: flex; flex-direction: column; gap: 10px; }
  .choice { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 12px 14px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); font: inherit; text-align: left; cursor: pointer; }
  .choice:disabled { opacity: .6; }
  .held { gap: 8px; margin-top: 8px; flex-wrap: wrap; }
  .small-choice { padding: 6px 10px; font-size: 13px; }
  .avatar-btn { position: relative; background: none; border: 0; padding: 0; cursor: pointer; }
  .badge { position: absolute; right: -1px; top: -1px; width: 10px; height: 10px; border-radius: 50%; background: var(--risk); border: 2px solid var(--surface); }
  .banners { padding: 0 16px 8px; }
  .content { flex: 1; min-height: 0; overflow: hidden; }
  .tabbar { flex: none; display: flex; justify-content: space-around; padding: 8px 0 calc(8px + var(--safe-bottom)); border-top: 1px solid var(--line); background: var(--surface); }
  .t { display: flex; flex-direction: column; align-items: center; gap: 3px; font-size: 10px; color: var(--ink-3); text-decoration: none; min-width: 64px; }
  .t .i { font-size: 20px; line-height: 1; }
  .t.on { color: var(--accent); }
  .menu { display: flex; flex-direction: column; }
  .menu a { padding: 14px 4px; text-decoration: none; border-top: 1px solid var(--line); font-size: 16px; }
  .menu a:first-child { border-top: 0; }
  .risk { color: var(--risk); font-weight: 700; }
  .awake { margin-top: 8px; }
  .sd { width: 8px; height: 8px; border-radius: 50%; background: var(--ok); }
  .sd.busy { background: var(--info); }
</style>
