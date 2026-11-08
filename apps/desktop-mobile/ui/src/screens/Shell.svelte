<script lang="ts">
  import Logo from '../components/Logo.svelte';
  import { api } from '../lib/api';
  import { go, param, section } from '../lib/route.svelte';
  import { app, fail, toast } from '../lib/store.svelte';
  import { plural } from '../lib/format';
  import { safetyLine } from '../lib/safety';
  import StatusChip from '../components/StatusChip.svelte';
  import { canPick, pickFiles, pickFolder } from '../lib/pick';
  import Banner from '../components/Banner.svelte';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';
  import GoogleImportSheet from '../components/GoogleImportSheet.svelte';
  import Library from './Library.svelte';
  import Albums from './Albums.svelte';
  import AlbumPage from './AlbumPage.svelte';
  import Deleted from './Deleted.svelte';
  import Backup from './Backup.svelte';
  import Settings from './Settings.svelte';
  import RecoveryKit from './RecoveryKit.svelte';
  import Search from './Search.svelte';
  import People from './People.svelte';
  import SearchField from '../components/SearchField.svelte';
  import Icon, { type IconName } from '../components/Icon.svelte';
  import { queryFromParam, searchPath } from '../lib/search';

  // Frequency decides placement (U1-11): the daily jobs on top, the albums in a group of their
  // own, and the rare ones (backup, trash, settings) in the footer.
  const top: { id: string; label: string; icon: IconName; key: string }[] = [
    { id: 'library', label: 'Photos', icon: 'photos', key: '1' },
    { id: 'favorites', label: 'Favorites', icon: 'heart', key: '2' },
    { id: 'people', label: 'People', icon: 'people', key: '3' },
  ];

  // ⌘1–⌘3 for the top items, ⌘, for Settings.
  function shortcuts(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return;
    if (e.key === ',') { e.preventDefault(); go('settings'); return; }
    const t = top.find((n) => n.key === e.key);
    if (t) { e.preventDefault(); go(t.id); }
  }
  let newAlbumOpen = $state(false);
  let newAlbumName = $state('');
  async function createAlbum() {
    try {
      const a = await api.createAlbum(newAlbumName);
      newAlbumName = '';
      newAlbumOpen = false;
      go(`album/${a.id}`);
    } catch (e) { fail(e); }
  }
  const line = $derived(safetyLine(app.backup_status, 'computer'));

  let importOpen = $state(false);
  $effect(() => { if (app.importRequest > 0) importOpen = true; });
  let importPath = $state('');
  let googleOpen = $state(false);
  let busy = $state(false);
  let kitOpen = $state(false);
  let kitDismissed = $state(false);
  // A replaced phrase matters most (the person's own copy is dead); then the newest alert.
  const alert = $derived(app.state?.alerts.find((a) => a.kind === 'phrase_replaced') ?? app.state?.alerts.at(-1));
  const recovery = $derived(app.state?.recovery_request ?? null);
  let answering = $state(false);
  async function answerRecovery(accept: boolean) {
    answering = true;
    try {
      await api.answerRecovery(accept);
      toast(accept ? 'The device was let in' : 'The recovery was refused');
    } catch (e) {
      fail(e);
    } finally {
      answering = false;
    }
  }
  const when = (ms: number) => new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
  const sec = $derived(section());
  // The toolbar's search field follows the query on the search screen.
  let searchText = $state('');
  $effect(() => {
    searchText = sec === 'search' ? queryFromParam(param()) : '';
  });
  function search(q: string) {
    if (q) go(searchPath(q));
    else if (sec === 'search') go('library');
  }
  const counts = $derived(app.counts);
  const atRisk = $derived(counts?.at_risk ?? 0);

  async function doImport(paths: string[]) {
    if (paths.length === 0) return;
    busy = true;
    try {
      const r = await api.importPaths(paths);
      const why = r.failed.length ? ` Why: ${r.failed[0].replace(/^.*?: /, '')}` : '';
      toast(`Imported ${plural(r.imported, 'photo')}${r.duplicates ? `, ${plural(r.duplicates, 'duplicate')} skipped` : ''}${r.skipped ? `, ${plural(r.skipped, 'file')} not a photo or video` : ''}${r.failed.length ? `, ${r.failed.length} failed.${why}` : ''}`);
      importOpen = false;
      importPath = '';
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  const title = $derived(
    sec === 'album' ? '' : { library: 'Photos', favorites: 'Favorites', albums: 'Albums', album: 'Album', shared: 'Shared albums', deleted: 'Recently Deleted', backup: 'Backup', settings: 'Settings', search: 'Search', similar: 'Similar photos', people: 'People', person: 'Person' }[sec] ?? 'Library',
  );
</script>

<svelte:window onkeydown={shortcuts} />

<div class="shell">
  <nav class="sidebar" aria-label="Sections">
    <div class="brand"><Logo size={22} />Shoebox</div>
    {#each top as n}
      <a class="it" class:on={sec === n.id || (n.id === 'people' && sec === 'person')} href="#/{n.id}" title="{n.label} (⌘{n.key})">
        <span class="ni"><Icon name={n.icon} size={18} /></span>{n.label}
        {#if n.id === 'library' && counts}<span class="n">{counts.photos.toLocaleString('en')}</span>{/if}
        {#if n.id === 'favorites' && counts}<span class="n">{counts.favorites}</span>{/if}
      </a>
    {/each}
    <a class="grp link" class:on={sec === 'albums'} href="#/albums">Albums</a>
    <div class="albums">
      {#each app.albums as a (a.id)}
        <a class="it" class:on={sec === 'album' && (param() === a.id || param() === a.sharedId)} href="#/album/{a.id}" title={a.name}>
          <span class="ni"><Icon name="albums" size={18} /></span><span class="nm">{a.name}</span>
          {#if a.shared}<span class="pill">Shared</span>{/if}
          {#if a.unread > 0}<span class="badge" aria-label="{a.unread} new">{a.unread}</span>{/if}
        </a>
      {/each}
      <button class="it new" onclick={() => (newAlbumOpen = true)}><span class="ni"><Icon name="add" size={18} /></span>New album</button>
    </div>
    <div class="foot">
      <StatusChip variant="row" onclick={() => go('backup')} />
      <a class="it" class:on={sec === 'deleted'} href="#/deleted"><span class="ni"><Icon name="trash" size={18} /></span>Recently Deleted{#if counts && counts.deleted}<span class="n">{counts.deleted}</span>{/if}</a>
      <a class="it" class:on={sec === 'settings'} href="#/settings" title="Settings (⌘,)"><span class="ni"><Icon name="settings" size={18} /></span>Settings</a>
      <div class="small muted device">{app.state?.device_name} · {app.state?.role}</div>
    </div>
  </nav>

  <main class="main">
    <header class="toolbar">
      <h3>{title}</h3>
      <span class="grow"></span>
      <div class="searchbox"><SearchField bind:value={searchText} onsubmit={search} oncancel={() => sec === 'search' && go('library')} shortcut /></div>
      {#if app.importProgress}<span class="small muted">Importing {app.importProgress.done} / {app.importProgress.total}…</span>{/if}
      <Button size="sm" onclick={() => (importOpen = true)}><Icon name="add" size={16} /> Import</Button>
    </header>

    {#if app.removed || app.state?.membership === 'removed'}
      <div class="banners"><Banner tone="risk" icon="block">This device was removed from the library. It no longer syncs; your photos on it stay until you delete them.</Banner></div>
    {:else if alert?.kind === 'phrase_replaced'}
      <div class="banners"><Banner tone="risk" icon="shield" action="Review" onaction={() => go('backup/devices')} ondismiss={() => api.dismissAlert(alert.device_id).catch(fail)}>Your library's <b>recovery phrase was replaced</b> on another device ({when(alert.at_ms)}). Your old phrase no longer works; get the new one from whoever replaced it. If nobody you know did, check Devices for devices you do not know.</Banner></div>
    {:else if alert}
      <div class="banners"><Banner tone="risk" icon="shield" action="Review" onaction={() => go('backup/devices')} ondismiss={() => api.dismissAlert(alert.device_id).catch(fail)}><b>{alert.device_name}</b> became an admin of your library ({when(alert.at_ms)}). If that was not you, someone may have your recovery phrase: remove that device in Devices.</Banner></div>
    {:else if app.state?.update}
      <div class="banners"><Banner tone="info" icon="download" action="Download" onaction={() => window.open(app.state!.update!.page, '_blank')} ondismiss={() => api.dismissUpdate().catch(fail)}><b>Shoebox {app.state.update.version}</b> is available. Download it and install it over this one: your library and settings stay.</Banner></div>
    {:else if app.archive?.state === 'stopped'}
      <div class="banners"><Banner tone="info" icon="download" action="Continue" onaction={() => (googleOpen = true)}>A Google Photos import did not finish. What it had done is kept.</Banner></div>
    {:else if line.banner && sec !== 'backup'}
      <div class="banners"><Banner tone={line.tone === 'risk' ? 'risk' : 'warn'} icon={line.tone === 'risk' ? 'warning' : 'pause'} action="View" onaction={() => go('backup')}>{line.long}</Banner></div>
    {:else if app.state && !app.state.recovery_kit_saved && !kitDismissed}
      <div class="banners"><Banner tone="accent" icon="key" action="I saved it" onaction={() => (kitOpen = true)} ondismiss={() => (kitDismissed = true)}>Your recovery kit is the only way back if every device is lost. You have not confirmed saving it.</Banner></div>
    {/if}

    <div class="content">
      {#if sec === 'library' || sec === ''}<Library source="all" />
      {:else if sec === 'favorites'}<Library source="favorites" />
      {:else if sec === 'albums'}<Albums />
      {:else if sec === 'album'}{#key param()}<AlbumPage id={param()!} />{/key}
      {:else if sec === 'deleted'}<Deleted />
      {:else if sec === 'backup'}<Backup />
      {:else if sec === 'settings'}<Settings />
      {:else if sec === 'search'}<Search />
      {:else if sec === 'similar'}{#key param()}<Search similar={param()} />{/key}
      {:else if sec === 'people'}<People />
      {:else if sec === 'person'}{#key param()}<Library source="person" photo={param()} />{/key}
      {:else}<Library source="all" />{/if}
    </div>
  </main>
</div>

<Sheet open={importOpen} title="Import photos" onclose={() => (importOpen = false)} width={440}>
  <div class="stack">
    <p class="muted small">Copies already in the library are skipped. Your originals stay where they are.</p>
    {#if canPick()}
      <Button kind="primary" block disabled={busy} onclick={async () => doImport(await pickFiles().catch((e) => { fail(e); return []; }))}>{busy ? 'Importing…' : 'Choose photos and videos…'}</Button>
      <Button block disabled={busy} onclick={async () => { const d = await pickFolder('Import a folder').catch((e) => { fail(e); return null; }); if (d) doImport([d]); }}>Choose a folder…</Button>
    {/if}
    <Button block disabled={busy} onclick={() => { importOpen = false; googleOpen = true; }}>From Google Photos…</Button>
    {#if !canPick()}
      <input class="input" placeholder="/Users/you/Pictures" bind:value={importPath} onkeydown={(e) => e.key === 'Enter' && doImport([importPath.trim()])} />
      <Button kind="primary" block disabled={busy || !importPath.trim()} onclick={() => doImport([importPath.trim()])}>{busy ? 'Importing…' : 'Import'}</Button>
    {/if}
  </div>
</Sheet>

<GoogleImportSheet open={googleOpen} onclose={() => (googleOpen = false)} />

<Sheet open={newAlbumOpen} title="New album" onclose={() => (newAlbumOpen = false)} width={380}>
  <form class="stack" onsubmit={(e) => { e.preventDefault(); createAlbum(); }}>
    <input class="input" placeholder="Album name" aria-label="Album name" bind:value={newAlbumName} />
    <Button type="submit" kind="primary" block disabled={!newAlbumName.trim()}>Create album</Button>
  </form>
</Sheet>

<Sheet open={kitOpen} title="Recovery kit" onclose={() => (kitOpen = false)} width={520}>
  <RecoveryKit onsaved={() => (kitOpen = false)} />
</Sheet>

<Sheet open={!!recovery} title="Let this device in?" onclose={() => answerRecovery(false)} width={420}>
  {#if recovery}
    <div class="stack">
      <p class="small">Someone is rejoining your library with your recovery phrase, through this device, as <b>{recovery.device_name}</b> ({recovery.role}). It would become an admin and see every photo.</p>
      <p class="small muted">Only allow this if it is you, setting up a device right now. If it is not, your recovery phrase is known to someone else.</p>
      <Button kind="primary" block disabled={answering} onclick={() => answerRecovery(true)}>Allow</Button>
      <Button kind="danger" block disabled={answering} onclick={() => answerRecovery(false)}>Refuse</Button>
    </div>
  {/if}
</Sheet>

<style>
  .shell { display: flex; height: 100%; background: var(--surface); }
  .sidebar { width: 220px; flex: none; background: var(--surface-2); border-right: 1px solid var(--line); padding: 14px 10px; display: flex; flex-direction: column; overflow: auto; }
  .brand { font-weight: 700; font-size: 15px; padding: 4px 8px 8px; display: flex; align-items: center; gap: 8px; }
  .grp { font-size: var(--t-xs); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.05em; margin: 16px 8px 6px; }
  .it { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 7px; font-size: 13.5px; text-decoration: none; }
  .it:hover { background: var(--surface-3); }
  .it.on { background: var(--surface-3); font-weight: 600; }
  .it .n { margin-left: auto; color: var(--ink-3); font-size: var(--t-sm); }
  .grp.link { text-decoration: none; display: block; }
  .grp.link:hover, .grp.link.on { color: var(--ink); }
  .albums { display: grid; gap: 1px; }
  .it .nm { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .it .pill { font-size: var(--t-xs); padding: 1px 6px; border-radius: 99px; background: var(--accent-soft); color: var(--accent); font-weight: 600; flex: none; }
  .it .badge { min-width: 18px; padding: 0 5px; text-align: center; border-radius: 99px; background: var(--accent); color: var(--accent-ink); font-size: var(--t-xs); font-weight: 700; flex: none; }
  .it.new { border: 0; background: none; font: inherit; font-size: 13.5px; color: var(--ink-2); cursor: pointer; width: 100%; text-align: left; }
  .foot { margin-top: auto; padding: 10px 0 0; display: grid; gap: 2px; }
  .foot .device { padding: 8px 8px 0; }
  .main { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .toolbar { height: 52px; flex: none; display: flex; align-items: center; gap: 12px; padding: 0 18px; border-bottom: 1px solid var(--line); }
  .toolbar h3 { margin: 0; font-size: 17px; }
  .searchbox { width: min(320px, 34vw); }
  .banners { padding: 10px 18px 0; }
  .content { flex: 1; min-height: 0; overflow: hidden; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; width: 100%; }
</style>
