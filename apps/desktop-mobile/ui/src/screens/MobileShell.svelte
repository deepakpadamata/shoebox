<script lang="ts">
  // The phone layout (U1-09): a large title and four tabs, Photos · Albums · Search · Settings.
  // Backup is the first section of Settings, and the chip in the Photos header opens it. Screens
  // are the same ones the desktop uses.
  import { api } from '../lib/api';
  import { go, param, rest, section } from '../lib/route.svelte';
  import { groupTitle } from './settings/groups';
  import { app, fail, refreshState, toast, ui } from '../lib/store.svelte';
  import { plural } from '../lib/format';
  import Icon, { type IconName } from '../components/Icon.svelte';
  import StatusChip from '../components/StatusChip.svelte';
  import { safetyLine } from '../lib/safety';
  import Banner from '../components/Banner.svelte';
  import Sheet from '../components/Sheet.svelte';
  import Toggle from '../components/Toggle.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import { hasKeepAwake, keepAwake } from '../lib/host';
  import Library from './Library.svelte';
  import Albums from './Albums.svelte';
  import AlbumPage from './AlbumPage.svelte';
  import Deleted from './Deleted.svelte';
  import Backup from './Backup.svelte';
  import Settings from './Settings.svelte';
  import RecoveryKit from './RecoveryKit.svelte';
  import Search from './Search.svelte';
  import People from './People.svelte';

  const tabs: { id: string; label: string; icon: IconName }[] = [
    { id: 'library', label: 'Photos', icon: 'photos' },
    { id: 'albums', label: 'Albums', icon: 'albums' },
    { id: 'search', label: 'Search', icon: 'search' },
    { id: 'settings', label: 'Settings', icon: 'settings' },
  ];
  const sec = $derived(section() || 'library');
  // Which tab a screen belongs to: Favorites, People and Recently Deleted hang off Albums,
  // Backup off Settings.
  const tab = $derived(
    ['albums', 'album', 'people', 'person', 'favorites', 'deleted'].includes(sec) ? 'albums' : ['backup', 'settings'].includes(sec) ? 'settings' : ['search', 'similar'].includes(sec) ? 'search' : 'library',
  );
  const line = $derived(safetyLine(app.backup_status, 'phone'));
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
  let awake = $state(false);
  let kitDismissed = $state(false);
  let visitedAlbums = $state(false);
  $effect(() => { if (sec === 'albums') visitedAlbums = true; });
  // Titles of the screens that are tabs or have a large title; sub-pages show a back link.
  const titles: Record<string, string> = { library: 'Photos', albums: 'Albums', search: 'Search', settings: 'Settings', favorites: 'Favorites', deleted: 'Recently Deleted', people: 'People', person: 'Person', backup: 'Backup' };
  const backTo: Record<string, string> = { favorites: 'Albums', deleted: 'Albums', people: 'Albums', person: 'People', album: 'Albums', backup: 'Settings' };
  const backPath: Record<string, string> = { favorites: 'albums', deleted: 'albums', people: 'albums', person: 'people', album: 'albums', backup: 'settings' };
  // Back goes to where the person came from; when there is no history (a link), to the parent.
  function back() {
    if (history.length > 1) history.back();
    else go(sec === 'settings' ? 'settings' : (backPath[sec] ?? 'library'));
  }
</script>

<div class="phone">
  {#if sec === 'similar'}
    <header class="navbar"><div class="grow"><button class="back" onclick={() => history.back()}><Icon name="back" size={18} /> Back</button></div></header>
    <main class="content">{#key param()}<Search similar={param()} />{/key}</main>
  {:else}
  {#if sec !== 'search'}
  <header class="navbar">
    <div class="grow">
      {#if backTo[sec] || (sec === 'settings' && rest()[0])}
        <button class="back" onclick={back}><Icon name="back" size={18} /> {sec === 'settings' ? 'Settings' : backTo[sec]}</button>
      {/if}
      {#if sec !== 'album'}<h2>{sec === 'settings' && groupTitle(rest()[0]) ? groupTitle(rest()[0]) : (titles[sec] ?? 'Photos')}</h2>{/if}
      {#if sec === 'library' || sec === ''}
        <StatusChip onclick={() => go('backup')} />
      {/if}
    </div>
    {#if sec === 'albums'}
      <button class="add-btn" aria-label="New album or join one" onclick={() => ui.albumsMenu++}><Icon name="add" size={22} /></button>
    {/if}
    {#if canAdd && (sec === 'library' || sec === '')}
      <button class="add-btn" aria-label="Add photos" onclick={() => (addOpen = true)}><Icon name="add" size={22} /></button>
    {/if}
  </header>
  {/if}

  {#if (app.backup?.gallery_deletions_held ?? 0) > 0}
    <div class="banners">
      <Banner tone="warn" icon="trash">
        <b>{plural(app.backup?.gallery_deletions_held ?? 0, 'photo')}</b> disappeared from this phone's gallery at once. Delete them from your library on every device too?
        <div class="row-flex held">
          <button class="choice small-choice" onclick={() => answerHeld(true)}>Delete them</button>
          <button class="choice small-choice" onclick={() => answerHeld(false)}>Keep them in the library</button>
        </div>
      </Banner>
    </div>
  {/if}

  {#if app.removed || app.state?.membership === 'removed'}
    <div class="banners"><Banner tone="risk" icon="block">This device was removed from the library.</Banner></div>
  {:else if line.banner && tab === 'library'}
    <div class="banners">
      <Banner tone={line.tone === 'risk' ? 'risk' : 'warn'} icon={line.tone === 'risk' ? 'warning' : 'pause'} action="View" onaction={() => go('backup')}>
        {line.long}
        {#if app.backupProgress}<ProgressBar total={app.backupProgress.total} segments={[{ value: app.backupProgress.done, tone: 'info' }]} label="Backing up" />{/if}
        {#if hasKeepAwake()}<label class="row-flex small awake"><Toggle checked={awake} label="Keep the screen on" onchange={(v) => { awake = v; keepAwake(v); }} /> Keep the screen on until done</label>{/if}
      </Banner>
    </div>
  {:else if app.backup && app.backup.permission !== 'unavailable' && !app.backup.policy.enabled && tab === 'library'}
    <div class="banners"><Banner tone="info" icon="cloud" action="Turn on" onaction={() => go('settings')}>Back up your camera roll to your other devices.</Banner></div>
  {:else if app.state && !app.state.recovery_kit_saved && !kitDismissed && tab === 'library'}
    <div class="banners"><Banner tone="accent" icon="key" action="I saved it" onaction={() => (kitOpen = true)} ondismiss={() => (kitDismissed = true)}>Confirm you saved your recovery kit.</Banner></div>
  {/if}

  <!-- The Photos and Albums tabs stay mounted once opened, hidden rather than removed, so each
       keeps its scroll position when you come back to it. -->
  <main class="content">
    <div class="pane" class:off={!(sec === 'library' || sec === '')}><Library source="all" /></div>
    {#if visitedAlbums}<div class="pane" class:off={sec !== 'albums'}><Albums /></div>{/if}
    {#if sec === 'favorites'}<Library source="favorites" />
    {:else if sec === 'album'}{#key param()}<AlbumPage id={param()!} />{/key}
    {:else if sec === 'people'}<People />
    {:else if sec === 'person'}{#key param()}<Library source="person" photo={param()} />{/key}
    {:else if sec === 'deleted'}<Deleted />
    {:else if sec === 'backup'}<Backup />
    {:else if sec === 'settings'}<Settings />
    {:else if sec === 'search'}<Search mobile />{/if}
  </main>

  <nav class="tabbar" aria-label="Sections">
    {#each tabs as t}
      <a class="t" class:on={tab === t.id} href="#/{t.id}" aria-current={tab === t.id ? 'page' : undefined}>
        <span class="i"><Icon name={t.icon} size={24} />{#if t.id === 'settings' && line.banner}<span class="dot" class:risk={line.tone === 'risk'}></span>{/if}</span>{t.label}
      </a>
    {/each}
  </nav>
  {/if}
</div>

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
  .add-btn { width: 36px; height: 36px; border-radius: 50%; border: 0; background: var(--accent-soft); color: var(--accent); display: grid; place-items: center; cursor: pointer; }
  .add { display: flex; flex-direction: column; gap: 10px; }
  .choice { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 12px 14px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); font: inherit; text-align: left; cursor: pointer; }
  .choice:disabled { opacity: .6; }
  .held { gap: 8px; margin-top: 8px; flex-wrap: wrap; }
  .small-choice { padding: 6px 10px; font-size: 13px; }
  .banners { padding: 0 16px 8px; }
  .content { flex: 1; min-height: 0; overflow: hidden; position: relative; }
  .pane { position: absolute; inset: 0; }
  .pane.off { visibility: hidden; pointer-events: none; }
  .tabbar { flex: none; display: flex; justify-content: space-around; padding: 8px 0 calc(8px + var(--safe-bottom)); border-top: 1px solid var(--line); background: var(--surface); }
  .t { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; font-size: var(--t-xs); min-height: var(--tap); color: var(--ink-3); text-decoration: none; min-width: 64px; }
  .t .i { position: relative; line-height: 1; }
  .t .i .dot { position: absolute; right: -4px; top: -2px; width: 10px; height: 10px; border-radius: 50%; background: var(--warn); border: 2px solid var(--surface); }
  .t .i .dot.risk { background: var(--risk); }
  .t.on { color: var(--accent); }
  .risk { color: var(--risk); font-weight: 700; }
  .awake { margin-top: 8px; }
</style>
