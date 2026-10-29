<script lang="ts">
  // Share the selected photos: into one of your shared albums (a new one, too), or to other apps
  // through the system share sheet.
  import { api, type SharedAlbumInfo } from '../lib/api';
  import { plural } from '../lib/format';
  import { fail, toast } from '../lib/store.svelte';
  import Sheet from './Sheet.svelte';
  import Button from './Button.svelte';
  import Toggle from './Toggle.svelte';

  interface Props {
    open: boolean;
    ids: string[];
    /** Where the share button is: the macOS picker opens next to it. */
    anchor: () => DOMRect | undefined;
    onclose: () => void;
    /** After the photos went into a shared album. */
    ondone: () => void;
  }
  let { open, ids, anchor, onclose, ondone }: Props = $props();

  let albums = $state<SharedAlbumInfo[]>([]);
  let newName = $state('');
  let busy = $state('');
  // Adding to an album asks first when photos carry a location and the album says to ask.
  let ask = $state<{ album: string; count: number } | null>(null);
  let remember = $state(false);

  $effect(() => {
    if (!open) return;
    ask = null;
    busy = '';
    remember = false;
    api.sharedAlbums().then((all) => (albums = all.filter((a) => a.role !== 'viewer'))).catch(fail);
  });

  async function pick(album: string) {
    busy = album;
    try {
      const check = await api.sharedAddCheck(album, ids);
      if (check.with_location > 0 && check.policy === 'ask') ask = { album, count: check.with_location };
      else await add(album, null);
    } catch (e) { fail(e); } finally { if (!ask) busy = ''; }
  }

  async function add(album: string, location: string | null) {
    busy = album;
    try {
      const r = await api.sharedAdd(album, ids, location, remember);
      const name = albums.find((a) => a.id === album)?.name ?? 'the album';
      toast(`Added ${plural(r.added, 'photo')} to ${name}${r.location_removed ? ` (location removed from ${r.location_removed})` : ''}${r.skipped.length ? `. Left out ${r.skipped.length}: ${r.skipped[0]}` : ''}`);
      ondone();
    } catch (e) { fail(e); } finally { busy = ''; ask = null; }
  }

  async function createAndAdd() {
    if (!newName.trim()) return;
    busy = 'new';
    try {
      const a = await api.sharedCreate(newName.trim());
      albums = [...albums, a];
      newName = '';
      await pick(a.id);
    } catch (e) { fail(e); busy = ''; }
  }

  async function otherApps() {
    busy = 'apps';
    const r = anchor();
    try {
      await api.shareFiles(ids, r ? { x: r.left, y: r.top, w: r.width, h: r.height } : undefined);
      onclose();
    } catch (e) { fail(e); } finally { busy = ''; }
  }
</script>

<Sheet {open} title={ask ? 'Share the location?' : `Share ${plural(ids.length, 'photo')}`} {onclose} width={400}>
  {#if ask}
    <div class="stack">
      <p>{plural(ask.count, 'photo')} {ask.count === 1 ? 'was' : 'were'} taken at a known place. Everyone in the album would see where.</p>
      <Button kind="primary" block disabled={busy !== ask.album} onclick={() => add(ask!.album, 'remove')}>Remove the location</Button>
      <Button block disabled={busy !== ask.album} onclick={() => add(ask!.album, 'keep')}>Keep the location</Button>
      <label class="row-flex small"><Toggle checked={remember} label="Remember for this album" onchange={(v) => (remember = v)} /> Remember for this album</label>
    </div>
  {:else}
    <div class="stack">
      {#if albums.length}
        <div class="label">Shared albums</div>
        <div class="list">
          {#each albums as a (a.id)}
            <button class="row" disabled={busy !== ''} onclick={() => pick(a.id)}>
              <span class="grow">{a.name}</span>
              <span class="muted small">{busy === a.id ? 'Adding…' : plural(a.member_count, 'person', 'people')}</span>
            </button>
          {/each}
        </div>
      {/if}
      <form class="row-flex" onsubmit={(e) => { e.preventDefault(); createAndAdd(); }}>
        <input class="input grow" placeholder="New shared album…" bind:value={newName} disabled={busy !== ''} />
        <Button type="submit" kind="primary" disabled={!newName.trim() || busy !== ''}>{busy === 'new' ? 'Creating…' : 'Create'}</Button>
      </form>
      <Button block disabled={busy !== ''} onclick={otherApps}>{busy === 'apps' ? `Getting ${plural(ids.length, 'original')} ready…` : 'Other apps…'}</Button>
    </div>
  {/if}
</Sheet>

<style>
  .label { font-size: 12px; font-weight: 600; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .list { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 12px; overflow: hidden; }
  .row { display: flex; align-items: center; gap: 10px; padding: 11px 14px; border: 0; background: var(--surface); text-align: left; cursor: pointer; }
  .row + .row { border-top: 1px solid var(--line); }
  .row:hover:not(:disabled) { background: var(--surface-2); }
  .row:disabled { cursor: default; opacity: 0.6; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
</style>
