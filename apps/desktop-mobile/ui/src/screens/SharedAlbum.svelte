<script lang="ts">
  import Icon from '../components/Icon.svelte';
  // One shared album (mockups 4a–4e): photos, activity, people; add photos (with the location
  // question), invite, save to my library, settings.
  import { onMount, untrack } from 'svelte';
  import { api, blobUrl, type FeedEntry, type PhotoSummary, type SharedAlbumInfo, type SharedItemInfo, type SharedMemberInfo } from '../lib/api';
  import { dateTime, plural } from '../lib/format';
  import { app, fail, refreshAlbums, toast } from '../lib/store.svelte';
  import { go } from '../lib/route.svelte';
  import Button from '../components/Button.svelte';
  import Pill from '../components/Pill.svelte';
  import Viewer from './Viewer.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Sheet from '../components/Sheet.svelte';
  import Toggle from '../components/Toggle.svelte';

  // The body of a shared album's page (U1-19): photos, activity and people. The page around it
  // (name, Add photos, Share, ⋯) is `AlbumPage`; photos open in the main viewer (U1-20).
  let { album, settingsOpen = $bindable(false), info = $bindable(null) }: { album: string; settingsOpen?: boolean; info?: SharedAlbumInfo | null } = $props();

  let items = $state<SharedItemInfo[]>([]);
  let next = $state<string | null>(null);
  let total = $state(0);
  let members = $state<SharedMemberInfo[]>([]);
  let feed = $state<FeedEntry[]>([]);
  let tab = $state('photos');
  let selected = $state<string[]>([]);
  let viewing = $state<number | null>(null);
  let missing = $state(false);
  const photos = $derived<PhotoSummary[]>(items.map((it) => ({ id: it.id, taken_at: it.taken_at || it.added_at, favorite: false, is_video: it.is_video, live: false, copies: 0, backed_up: true, has_thumb: it.has_thumb, has_poster: false, needs_poster: false, has_original: it.has_original, width: 0, height: 0 })));

  const canAdd = $derived(info?.role === 'owner' || info?.role === 'contributor');
  const isOwner = $derived(info?.role === 'owner');
  const selecting = $derived(selected.length > 0);

  async function load() {
    const all = await api.sharedAlbums();
    info = all.find((a) => a.id === album) ?? null;
    missing = !info;
    if (!info) return;
    const page = await api.sharedItems(album, null, Math.max(500, items.length));
    items = page.items;
    next = page.next;
    total = page.total;
    members = await api.sharedMembers(album);
    feed = await api.sharedFeed(album);
  }
  onMount(() => { load().then(() => api.sharedSeen(album)).catch(fail); });
  // New photos or downloads arrive in the background.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    app.sharedVersion;
    untrack(() => {
      clearTimeout(timer);
      timer = setTimeout(() => load().then(() => viewing === null && tab === 'photos' && api.sharedSeen(album)).catch(() => {}), 250);
    });
  });

  async function more() {
    if (!next) return;
    const page = await api.sharedItems(album, next, 500);
    items = [...items, ...page.items];
    next = page.next;
  }

  function open(i: number) {
    if (selecting) toggle(items[i].id);
    else viewing = i;
  }
  function toggle(id: string) {
    selected = selected.includes(id) ? selected.filter((s) => s !== id) : [...selected, id];
  }

  async function save(ids: string[]) {
    try {
      const n = await api.sharedSave(album, ids);
      toast(`Saved ${plural(n, 'photo')} to your library`);
      selected = [];
    } catch (e) { fail(e); }
  }

  // ----- settings -----
  let leaveAsk = $state(false);
  let removing = $state<SharedMemberInfo | null>(null);
  async function removeMember() {
    const m = removing!;
    try {
      await api.sharedRemoveMember(album, m.key);
      toast(`${m.name} was removed`);
      removing = null;
      await load();
    } catch (e) { fail(e); }
  }
  async function setKeep(v: boolean) {
    try { await api.sharedSettings(album, v, null); await load(); } catch (e) { fail(e); }
  }
  async function setLocation(v: string) {
    try { await api.sharedSettings(album, null, v); await load(); } catch (e) { fail(e); }
  }
  async function leave() {
    try { await api.sharedLeave(album); await refreshAlbums(); go('albums'); } catch (e) { fail(e); }
  }

  const feedText = (f: FeedEntry) => (f.kind === 'joined' ? `${f.member} joined` : `${f.member} added ${plural(f.count, 'photo')}`);
</script>

{#if missing}
  <div class="page"><p class="muted">This album is not on this device any more.</p><Button onclick={() => go('albums')}>Back to albums</Button></div>
{:else if info}
  <div class="page">
    {#if selecting}
      <div class="selbar">
        <b>{selected.length} selected</b>
        <Button size="sm" kind="primary" onclick={() => save(selected)}>Save to my library</Button>
        <Button size="sm" kind="ghost" onclick={() => (selected = [])}>Cancel</Button>
      </div>
    {:else}
      <Segmented value={tab} onchange={(v) => (tab = v)} options={[{ value: 'photos', label: `Photos ${total}` }, { value: 'activity', label: 'Activity' }, { value: 'people', label: `People ${members.length}` }]} />
    {/if}

    {#if tab === 'photos' || selecting}
      {#if items.length === 0}
        <div class="empty">
          <p class="muted">{canAdd ? 'No photos yet. Add some, or invite people to add theirs.' : 'No photos yet. They show up here when someone adds them.'}</p>
        </div>
      {:else}
        <div class="grid">
          {#each items as it, i (it.id)}
            <button class="tile" class:sel={selected.includes(it.id)} onclick={() => open(i)} oncontextmenu={(e) => { e.preventDefault(); toggle(it.id); }} aria-label="Photo from {it.added_by}">
              {#if it.has_thumb && !it.is_video}<img src={blobUrl(it.id, 'thumb', album)} alt="" loading="lazy" decoding="async" />{:else}<span class="ph"></span>{/if}
              {#if it.is_video}<span class="vid"><Icon name="play" size={14} fill /></span>{/if}
              {#if selecting}<span class="check">{#if selected.includes(it.id)}<Icon name="check" size={14} />{/if}</span>{/if}
            </button>
          {/each}
        </div>
        {#if next}<div class="more"><Button onclick={more}>Show more</Button></div>{/if}
      {/if}
    {:else if tab === 'activity'}
      <div class="list">
        {#each feed as f}
          <div class="row"><span><Icon name={f.kind === 'joined' ? 'person' : 'photos'} size={18} /></span><span>{feedText(f)}</span><span class="grow"></span><span class="muted small">{dateTime(f.at)}</span></div>
        {:else}
          <p class="muted">Nothing yet.</p>
        {/each}
      </div>
    {:else}
      <div class="list">
        {#each members as m (m.key)}
          <div class="row"><Icon name="person" size={18} /><span>{m.name}{m.me ? ' (you)' : ''}{#if m.devices > 1}<span class="muted small"> · {m.devices} devices</span>{/if}</span><span class="grow"></span><Pill tone={m.role === 'owner' ? 'info' : 'mute'}>{m.role}</Pill>{#if isOwner && !m.me}<Button size="sm" kind="ghost" onclick={() => (removing = m)}>Remove</Button>{/if}</div>
        {/each}
        <p class="muted small">People who leave or are removed keep the photos they already downloaded.</p>
      </div>
    {/if}
  </div>
{/if}

{#if viewing !== null && photos[viewing]}
  <Viewer items={photos} index={viewing} onclose={() => (viewing = null)} onmove={(d) => (viewing = Math.min(photos.length - 1, Math.max(0, viewing! + d)))}
    shared={{ album, role: info?.role ?? 'viewer', meta: items, onsave: (id) => save([id]), onchange: () => load().catch(() => {}) }} />
{/if}

<Sheet open={removing !== null} title="Remove {removing?.name ?? ''}?" onclose={() => (removing = null)} width={420}>
  <div class="stack">
    <p>{removing?.name} will not see anything added from now on. Photos they already downloaded stay on their devices: that cannot be undone.</p>
    <p class="muted small">The album moves to a new private space and everyone else follows automatically, as soon as their devices are online.</p>
    <Button kind="danger" block onclick={removeMember}>Remove {removing?.name}</Button>
    <Button kind="ghost" block onclick={() => (removing = null)}>Cancel</Button>
  </div>
</Sheet>

<Sheet open={settingsOpen} title="Album settings" onclose={() => (settingsOpen = false)} width={440}>
  {#if info}
    <div class="stack">
      <div class="row-flex"><div class="grow"><b>Keep every original here</b><div class="muted small">Otherwise this device keeps previews and fetches an original when you open it.</div></div><Toggle checked={info.keep_originals} label="Keep originals" onchange={setKeep} /></div>
      {#if canAdd}
        <div>
          <div class="label">When adding photos with a location</div>
          <Segmented value={info.location} onchange={setLocation} options={[{ value: 'ask', label: 'Ask' }, { value: 'keep', label: 'Keep' }, { value: 'remove', label: 'Remove' }]} />
        </div>
      {/if}
      {#if !leaveAsk}
        <Button kind="danger" block onclick={() => (leaveAsk = true)}>Leave this album</Button>
      {:else}
        <p class="small">Leaving removes the album from this device. You keep photos you saved to your library.{isOwner ? ' You are the owner: other people keep the album and can still add photos, but nobody will be able to invite more people.' : ''}</p>
        <Button kind="danger" block onclick={leave}>Leave the album</Button>
        <Button kind="ghost" block onclick={() => (leaveAsk = false)}>Stay</Button>
      {/if}
    </div>
  {/if}
</Sheet>

<style>
  .page { padding: 14px 18px; overflow: auto; height: 100%; display: flex; flex-direction: column; gap: 12px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 2px; }
  .tile { position: relative; aspect-ratio: 1; border: 0; padding: 0; background: var(--surface-2); cursor: pointer; overflow: hidden; }
  .tile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .ph { position: absolute; inset: 0; background: linear-gradient(135deg, var(--surface-2), var(--surface-3)); }
  .vid { position: absolute; left: 5px; bottom: 4px; font-size: 10px; color: #fff; text-shadow: 0 1px 2px rgba(0,0,0,.6); }
  .check { position: absolute; right: 5px; top: 5px; width: 18px; height: 18px; border-radius: 50%; border: 2px solid #fff; background: rgba(0,0,0,.15); color: #fff; font-size: var(--t-xs); font-weight: 700; display: grid; place-items: center; }
  .sel .check { background: var(--accent); }
  .sel::after { content: ''; position: absolute; inset: 0; background: rgba(255,255,255,.25); pointer-events: none; }
  .more { text-align: center; padding: 10px; }
  .selbar { display: flex; align-items: center; gap: 10px; }
  .empty { margin-top: 24px; }
  .list { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 10px; padding: 10px 2px; border-top: 1px solid var(--line); }
  .label { font-size: var(--t-sm); font-weight: 600; color: var(--ink-3); margin-bottom: 6px; }
</style>
