<script lang="ts">
  // Shared albums (mockups 4a–4e): the list, creating one, joining with an invitation.
  import { onMount, untrack } from 'svelte';
  import { api, blobUrl, type SharedAlbumInfo } from '../lib/api';
  import { plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import { go, param } from '../lib/route.svelte';
  import { canScan, scanQr } from '../lib/scan';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';
  import SharedAlbum from './SharedAlbum.svelte';

  let albums = $state<SharedAlbumInfo[]>([]);
  let loaded = $state(false);
  let createOpen = $state(false);
  let joinOpen = $state(false);
  let name = $state('');
  let code = $state('');
  let busy = $state(false);
  const id = $derived(param());

  async function load() {
    albums = await api.sharedAlbums();
    loaded = true;
  }
  onMount(() => { load().catch(fail); });
  $effect(() => { app.sharedVersion; untrack(() => { load().catch(() => {}); }); });

  async function create() {
    busy = true;
    try {
      const a = await api.sharedCreate(name);
      name = '';
      createOpen = false;
      go(`shared/${a.id}`);
    } catch (e) { fail(e); } finally { busy = false; }
  }

  async function join() {
    busy = true;
    try {
      const a = await api.sharedJoin(code.trim());
      code = '';
      joinOpen = false;
      toast(`You joined “${a.name}”`);
      go(`shared/${a.id}`);
    } catch (e) { fail(e); } finally { busy = false; }
  }

  async function scan() {
    try {
      const text = await scanQr();
      if (text) code = text.trim();
    } catch (e) { fail(e); }
  }
</script>

{#if id}
  {#key id}<SharedAlbum album={id} />{/key}
{:else}
  <div class="page">
    <div class="row-flex">
      <Button kind="primary" onclick={() => (createOpen = true)}>＋ New shared album</Button>
      <Button onclick={() => (joinOpen = true)}>Join with an invitation</Button>
    </div>
    {#if loaded && albums.length === 0}
      <div class="empty">
        <b>Share photos with family and friends</b>
        <p class="muted">Make a shared album, invite people with a QR code, and everyone adds photos. Nothing goes through a company’s cloud: the photos travel between your devices.</p>
      </div>
    {:else}
      <div class="grid">
        {#each albums as a (a.id)}
          <a class="album" href="#/shared/{a.id}">
            <div class="cover">
              {#if a.cover}<img src={blobUrl(a.cover, 'thumb', a.id)} alt="" loading="lazy" />{:else}<span>👥</span>{/if}
              {#if a.unread > 0}<span class="dot" title="{plural(a.unread, 'new photo')}">{a.unread}</span>{/if}
            </div>
            <b>{a.name}</b>
            <span class="muted small">{plural(a.item_count, 'photo')} · {plural(a.member_count, 'person', 'people')}</span>
          </a>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<Sheet open={createOpen} title="New shared album" onclose={() => (createOpen = false)} width={400}>
  <form class="stack" onsubmit={(e) => { e.preventDefault(); create(); }}>
    <input class="input" placeholder="Lisbon trip" bind:value={name} />
    <p class="muted small">You will be the owner. Invite people from inside the album.</p>
    <Button type="submit" kind="primary" block disabled={busy || !name.trim()}>Create</Button>
  </form>
</Sheet>

<Sheet open={joinOpen} title="Join a shared album" onclose={() => (joinOpen = false)} width={440}>
  <div class="stack">
    <p class="muted small">Ask the owner to show you the invitation (a QR code or a code to paste). Both of you need to be online while you join.</p>
    {#if canScan()}<Button block onclick={scan}>Scan the QR code</Button>{/if}
    <textarea class="input mono" rows="3" placeholder="SHOEBOX-ALBUM1-…" bind:value={code}></textarea>
    <Button kind="primary" block disabled={busy || !code.trim()} onclick={join}>{busy ? 'Joining…' : 'Join'}</Button>
  </div>
</Sheet>

<style>
  .page { padding: 18px; overflow: auto; height: 100%; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 16px; margin-top: 18px; }
  .album { text-decoration: none; display: flex; flex-direction: column; gap: 2px; }
  .cover { position: relative; aspect-ratio: 1; border-radius: 12px; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; font-size: 34px; margin-bottom: 6px; }
  .cover img { width: 100%; height: 100%; object-fit: cover; }
  .dot { position: absolute; right: 8px; top: 8px; min-width: 20px; height: 20px; padding: 0 6px; border-radius: 10px; background: var(--accent); color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .empty { margin-top: 40px; max-width: 460px; }
  .input { border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px 12px; color: var(--ink); font: inherit; width: 100%; }
</style>
