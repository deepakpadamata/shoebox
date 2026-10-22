<script lang="ts">
  import { api } from '../lib/api';
  import { go, param, section } from '../lib/route.svelte';
  import { app, fail, toast } from '../lib/store.svelte';
  import { plural } from '../lib/format';
  import Banner from '../components/Banner.svelte';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';
  import Library from './Library.svelte';
  import Albums from './Albums.svelte';
  import Shared from './Shared.svelte';
  import Deleted from './Deleted.svelte';
  import Devices from './Devices.svelte';
  import Health from './Health.svelte';
  import Settings from './Settings.svelte';
  import RecoveryKit from './RecoveryKit.svelte';

  const nav = [
    { id: 'library', label: 'Library', icon: '▦' },
    { id: 'favorites', label: 'Favorites', icon: '♥' },
    { id: 'albums', label: 'Albums', icon: '▤' },
    { id: 'shared', label: 'Shared', icon: '👥' },
    { id: 'deleted', label: 'Recently Deleted', icon: '🗑' },
  ];
  const more = [
    { id: 'devices', label: 'Devices', icon: '⇄' },
    { id: 'health', label: 'Storage', icon: '◔' },
    { id: 'settings', label: 'Settings', icon: '⚙︎' },
  ];

  let importOpen = $state(false);
  $effect(() => { if (app.importRequest > 0) importOpen = true; });
  let importPath = $state('');
  let busy = $state(false);
  let kitOpen = $state(false);
  let kitDismissed = $state(false);
  let albumName = $state('');
  let renameOpen = $state(false);
  let renameTo = $state('');

  $effect(() => {
    app.version;
    const id = section() === 'album' ? param() : undefined;
    if (!id) return;
    api.albums().then((all) => (albumName = all.find((a) => a.id === id)?.name ?? 'Album')).catch(() => {});
  });

  async function rename() {
    try {
      await api.renameAlbum(param()!, renameTo);
      albumName = renameTo.trim();
      renameOpen = false;
    } catch (e) {
      fail(e);
    }
  }
  const sec = $derived(section());
  const counts = $derived(app.counts);
  const atRisk = $derived(counts?.at_risk ?? 0);

  async function doImport() {
    if (!importPath.trim()) return;
    busy = true;
    try {
      const r = await api.importPaths([importPath.trim()]);
      toast(`Imported ${plural(r.imported, 'photo')}${r.duplicates ? `, ${plural(r.duplicates, 'duplicate')} skipped` : ''}${r.failed.length ? `, ${r.failed.length} failed` : ''}`);
      importOpen = false;
      importPath = '';
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  const title = $derived(
    sec === 'album' && albumName ? albumName : { library: 'Library', favorites: 'Favorites', albums: 'Albums', album: 'Album', shared: 'Shared albums', deleted: 'Recently Deleted', devices: 'Devices', health: 'Storage', settings: 'Settings' }[sec] ?? 'Library',
  );
</script>

<div class="shell">
  <nav class="sidebar" aria-label="Sections">
    <div class="brand">📦 Shoebox</div>
    <div class="grp">Library</div>
    {#each nav as n}
      <a class="it" class:on={sec === n.id || (n.id === 'albums' && sec === 'album')} href="#/{n.id}">
        <span>{n.icon}</span>{n.label}
        {#if n.id === 'library' && counts}<span class="n">{counts.photos.toLocaleString('en')}</span>{/if}
        {#if n.id === 'favorites' && counts}<span class="n">{counts.favorites}</span>{/if}
        {#if n.id === 'deleted' && counts && counts.deleted}<span class="n">{counts.deleted}</span>{/if}
      </a>
    {/each}
    <div class="grp">This device</div>
    {#each more as n}
      <a class="it" class:on={sec === n.id} href="#/{n.id}"><span>{n.icon}</span>{n.label}
        {#if n.id === 'health' && atRisk > 0}<span class="n risk">{atRisk}</span>{/if}
      </a>
    {/each}
    <div class="foot small muted">{app.state?.device_name} · {app.state?.role}</div>
  </nav>

  <main class="main">
    <header class="toolbar">
      <h3>{title}</h3>
      <span class="grow"></span>
      {#if sec === 'album'}<Button size="sm" kind="ghost" onclick={() => { renameTo = albumName; renameOpen = true; }}>Rename</Button>{/if}
      <span class="status small muted" title={app.syncError}>
        <span class="sd" class:busy={app.syncing.length > 0} class:bad={!!app.syncError && app.syncing.length === 0}></span>
        {app.syncing.length > 0 ? 'Syncing…' : app.syncError ? 'Some devices are not reachable' : 'Up to date'}
      </span>
      {#if app.importProgress}<span class="small muted">Importing {app.importProgress.done} / {app.importProgress.total}…</span>{/if}
      <Button size="sm" onclick={() => (importOpen = true)}>＋ Import</Button>
    </header>

    {#if app.removed || app.state?.membership === 'removed'}
      <div class="banners"><Banner tone="risk" icon="⛔">This device was removed from the library. It no longer syncs; your photos on it stay until you delete them.</Banner></div>
    {:else if atRisk > 0 && sec !== 'health'}
      <div class="banners"><Banner tone="risk" action="View" onaction={() => go('health')}><b>{plural(atRisk, 'photo')}</b> {atRisk === 1 ? 'is' : 'are'} not backed up on enough devices yet.</Banner></div>
    {:else if app.state && !app.state.recovery_kit_saved && !kitDismissed}
      <div class="banners"><Banner tone="accent" icon="🔑" action="I saved it" onaction={() => (kitOpen = true)} ondismiss={() => (kitDismissed = true)}>Your recovery kit is the only way back if every device is lost. You have not confirmed saving it.</Banner></div>
    {/if}

    <div class="content">
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
    </div>
  </main>
</div>

<Sheet open={importOpen} title="Import photos" onclose={() => (importOpen = false)} width={440}>
  <div class="stack">
    <p class="muted small">Type or paste the path of a folder (or a photo) on this computer. Copies already in the library are skipped.</p>
    <input class="input" placeholder="/Users/you/Pictures" bind:value={importPath} onkeydown={(e) => e.key === 'Enter' && doImport()} />
    <Button kind="primary" block disabled={busy || !importPath.trim()} onclick={doImport}>{busy ? 'Importing…' : 'Import'}</Button>
  </div>
</Sheet>

<Sheet open={renameOpen} title="Rename album" onclose={() => (renameOpen = false)} width={380}>
  <form class="stack" onsubmit={(e) => { e.preventDefault(); rename(); }}>
    <input class="input" bind:value={renameTo} />
    <Button type="submit" kind="primary" block disabled={!renameTo.trim()}>Rename</Button>
  </form>
</Sheet>

<Sheet open={kitOpen} title="Recovery kit" onclose={() => (kitOpen = false)} width={520}>
  <RecoveryKit onsaved={() => (kitOpen = false)} />
</Sheet>

<style>
  .shell { display: flex; height: 100%; background: var(--surface); }
  .sidebar { width: 220px; flex: none; background: var(--surface-2); border-right: 1px solid var(--line); padding: 14px 10px; display: flex; flex-direction: column; overflow: auto; }
  .brand { font-weight: 700; font-size: 15px; padding: 4px 8px 8px; }
  .grp { font-size: 11px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.05em; margin: 16px 8px 6px; }
  .it { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 7px; font-size: 13.5px; text-decoration: none; }
  .it:hover { background: var(--surface-3); }
  .it.on { background: var(--surface-3); font-weight: 600; }
  .it .n { margin-left: auto; color: var(--ink-3); font-size: 12px; }
  .it .n.risk { color: var(--risk); font-weight: 700; }
  .foot { margin-top: auto; padding: 10px 8px 0; }
  .main { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .toolbar { height: 52px; flex: none; display: flex; align-items: center; gap: 12px; padding: 0 18px; border-bottom: 1px solid var(--line); }
  .toolbar h3 { margin: 0; font-size: 17px; }
  .banners { padding: 10px 18px 0; }
  .status { display: inline-flex; align-items: center; gap: 6px; }
  .sd { width: 8px; height: 8px; border-radius: 50%; background: var(--ok); }
  .sd.busy { background: var(--info); }
  .sd.bad { background: var(--warn); }
  .content { flex: 1; min-height: 0; overflow: hidden; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; width: 100%; }
</style>
