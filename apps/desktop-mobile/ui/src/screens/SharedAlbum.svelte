<script lang="ts">
  // One shared album (mockups 4a–4e): photos, activity, people; add photos (with the location
  // question), invite, save to my library, settings.
  import { onMount, untrack } from 'svelte';
  import { api, blobUrl, videoUrl, type FeedEntry, type InviteOut, type PhotoSummary, type SharedAlbumInfo, type SharedItemInfo, type SharedMemberInfo } from '../lib/api';
  import { dateTime, plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import { go } from '../lib/route.svelte';
  import Button from '../components/Button.svelte';
  import Pill from '../components/Pill.svelte';
  import QR from '../components/QR.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Sheet from '../components/Sheet.svelte';
  import Toggle from '../components/Toggle.svelte';

  let { album }: { album: string } = $props();

  let info = $state<SharedAlbumInfo | null>(null);
  let items = $state<SharedItemInfo[]>([]);
  let next = $state<string | null>(null);
  let total = $state(0);
  let members = $state<SharedMemberInfo[]>([]);
  let feed = $state<FeedEntry[]>([]);
  let tab = $state('photos');
  let selected = $state<string[]>([]);
  let viewing = $state<number | null>(null);
  let missing = $state(false);

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

  // ----- adding -----
  let addOpen = $state(false);
  let library = $state<PhotoSummary[]>([]);
  let picked = $state<string[]>([]);
  let askOpen = $state(false);
  let withLocation = $state(0);
  let remember = $state(false);
  let adding = $state(false);

  async function openAdd() {
    addOpen = true;
    picked = [];
    library = (await api.libraryPage(null, 600).catch(() => ({ items: [], next: null }))).items;
  }
  async function addPicked() {
    try {
      const check = await api.sharedAddCheck(album, picked);
      if (check.with_location > 0 && check.policy === 'ask') {
        withLocation = check.with_location;
        askOpen = true;
        return;
      }
      await doAdd(null);
    } catch (e) { fail(e); }
  }
  async function doAdd(location: string | null) {
    adding = true;
    try {
      const r = await api.sharedAdd(album, picked, location, remember);
      toast(`Added ${plural(r.added, 'photo')}${r.location_removed ? ` (location removed from ${r.location_removed})` : ''}${r.skipped.length ? `. Left out ${r.skipped.length}: ${r.skipped[0]}` : ''}`);
      addOpen = false;
      askOpen = false;
      picked = [];
      await load();
    } catch (e) { fail(e); } finally { adding = false; }
  }

  // ----- inviting -----
  let inviteOpen = $state(false);
  let role = $state('contributor');
  let invite = $state<InviteOut | null>(null);
  async function makeInvite() {
    try { invite = await api.sharedInvite(album, role, 7); } catch (e) { fail(e); }
  }
  async function copy() {
    try { await navigator.clipboard.writeText(invite!.text); toast('Invitation copied'); } catch { toast('Select the text and copy it'); }
  }

  // ----- settings -----
  let settingsOpen = $state(false);
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
    try { await api.sharedLeave(album); go('shared'); } catch (e) { fail(e); }
  }

  // ----- viewer -----
  let fullLoading = $state(false);
  let full = $state(false);
  $effect(() => { viewing; full = false; });
  async function showOriginal(it: SharedItemInfo) {
    fullLoading = true;
    try {
      await api.sharedFetch(album, it.id, 'original');
      full = true;
      await load();
    } catch (e) { fail(e); } finally { fullLoading = false; }
  }
  function key(e: KeyboardEvent) {
    if (viewing === null) return;
    if (e.key === 'Escape') viewing = null;
    else if (e.key === 'ArrowRight' && viewing < items.length - 1) viewing++;
    else if (e.key === 'ArrowLeft' && viewing > 0) viewing--;
  }
  const feedText = (f: FeedEntry) => (f.kind === 'joined' ? `${f.member} joined` : `${f.member} added ${plural(f.count, 'photo')}`);
  const current = $derived(viewing !== null ? items[viewing] : null);
</script>

<svelte:window onkeydown={key} />

{#if missing}
  <div class="page"><p class="muted">This album is not on this device any more.</p><Button onclick={() => go('shared')}>Back to shared albums</Button></div>
{:else if info}
  <div class="page">
    <div class="head">
      <a class="back" href="#/shared">‹ Shared</a>
      <h2>{info.name}</h2>
      <Pill tone={isOwner ? 'info' : 'mute'}>{info.role}</Pill>
      <span class="grow"></span>
      {#if canAdd}<Button kind="primary" size="sm" onclick={openAdd}>＋ Add photos</Button>{/if}
      {#if isOwner}<Button size="sm" onclick={() => { inviteOpen = true; invite = null; }}>Invite</Button>{/if}
      <Button size="sm" kind="ghost" onclick={() => (settingsOpen = true)} title="Album settings">⚙︎</Button>
    </div>

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
          {#if canAdd}<Button kind="primary" onclick={openAdd}>Add photos</Button>{/if}
          {#if isOwner}<Button onclick={() => { inviteOpen = true; invite = null; }}>Invite people</Button>{/if}
        </div>
      {:else}
        <div class="grid">
          {#each items as it, i (it.id)}
            <button class="tile" class:sel={selected.includes(it.id)} onclick={() => open(i)} oncontextmenu={(e) => { e.preventDefault(); toggle(it.id); }} aria-label="Photo from {it.added_by}">
              {#if it.has_thumb && !it.is_video}<img src={blobUrl(it.id, 'thumb', album)} alt="" loading="lazy" decoding="async" />{:else}<span class="ph"></span>{/if}
              {#if it.is_video}<span class="vid">▶</span>{/if}
              {#if selecting}<span class="check">{selected.includes(it.id) ? '✓' : ''}</span>{/if}
            </button>
          {/each}
        </div>
        {#if next}<div class="more"><Button onclick={more}>Show more</Button></div>{/if}
      {/if}
    {:else if tab === 'activity'}
      <div class="list">
        {#each feed as f}
          <div class="row"><span>{f.kind === 'joined' ? '👋' : '🖼'}</span><span>{feedText(f)}</span><span class="grow"></span><span class="muted small">{dateTime(f.at)}</span></div>
        {:else}
          <p class="muted">Nothing yet.</p>
        {/each}
      </div>
    {:else}
      <div class="list">
        {#each members as m (m.key)}
          <div class="row"><span>👤</span><span>{m.name}{m.me ? ' (you)' : ''}{#if m.devices > 1}<span class="muted small"> · {m.devices} devices</span>{/if}</span><span class="grow"></span><Pill tone={m.role === 'owner' ? 'info' : 'mute'}>{m.role}</Pill>{#if isOwner && !m.me}<Button size="sm" kind="ghost" onclick={() => (removing = m)}>Remove</Button>{/if}</div>
        {/each}
        <p class="muted small">People who leave or are removed keep the photos they already downloaded.</p>
      </div>
    {/if}
  </div>
{/if}

{#if current}
  <div class="viewer" role="dialog" aria-modal="true" aria-label="Photo">
    <div class="vbar">
      <button class="vb" onclick={() => (viewing = null)}>‹ Back</button>
      <span class="grow small">{current.added_by} · {dateTime(current.taken_at || current.added_at)}{current.location_removed ? ' · location removed' : ''}</span>
      <button class="vb" onclick={() => save([current.id])}>Save to library</button>
      {#if !current.has_original && !current.is_video}<button class="vb" disabled={fullLoading} onclick={() => showOriginal(current)}>{fullLoading ? 'Downloading…' : 'Get original'}</button>{/if}
    </div>
    <div class="stage">
      {#key current.id + (full ? 'f' : 'p')}
        {#if current.is_video && current.has_original}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={videoUrl(current.id, album)} controls></video>
        {:else if current.has_original && !current.is_video}
          <img src={blobUrl(current.id, 'original', album)} alt="" />
        {:else if current.is_video}
          <div class="muted">Videos play once the original is downloaded.</div>
          <Button onclick={() => showOriginal(current)} disabled={fullLoading}>{fullLoading ? 'Downloading…' : 'Download video'}</Button>
        {:else}
          <img src={blobUrl(current.id, 'preview', album)} alt="" />
        {/if}
      {/key}
      {#if viewing! > 0}<button class="nav l" aria-label="Previous" onclick={() => (viewing = viewing! - 1)}>‹</button>{/if}
      {#if viewing! < items.length - 1}<button class="nav r" aria-label="Next" onclick={() => (viewing = viewing! + 1)}>›</button>{/if}
    </div>
  </div>
{/if}

<Sheet open={addOpen} title="Add photos to {info?.name ?? 'the album'}" onclose={() => (addOpen = false)} width={680}>
  <div class="stack">
    <p class="muted small">Choose from your library. {picked.length ? `${plural(picked.length, 'photo')} chosen.` : ''}</p>
    <div class="pick">
      {#each library as p (p.id)}
        <button class="ptile" class:sel={picked.includes(p.id)} onclick={() => (picked = picked.includes(p.id) ? picked.filter((x) => x !== p.id) : [...picked, p.id])} aria-label="Photo">
          {#if p.has_thumb && !p.is_video}<img src={blobUrl(p.id, 'thumb')} alt="" loading="lazy" />{/if}
          <span class="check">{picked.includes(p.id) ? '✓' : ''}</span>
        </button>
      {/each}
    </div>
    <Button kind="primary" block disabled={!picked.length || adding} onclick={addPicked}>{adding ? 'Adding…' : `Add ${plural(picked.length, 'photo')}`}</Button>
  </div>
</Sheet>

<Sheet open={askOpen} title="Share the location?" onclose={() => (askOpen = false)} width={420}>
  <div class="stack">
    <p>{plural(withLocation, 'photo')} {withLocation === 1 ? 'was' : 'were'} taken at a known place. Everyone in this album would see where.</p>
    <Button kind="primary" block disabled={adding} onclick={() => doAdd('remove')}>Remove the location</Button>
    <Button block disabled={adding} onclick={() => doAdd('keep')}>Keep the location</Button>
    <label class="row-flex small"><Toggle checked={remember} label="Remember for this album" onchange={(v) => (remember = v)} /> Remember for this album</label>
    <p class="muted small">Removing the location changes only the shared copy; your own photo keeps it. Some formats (such as HEIC) cannot be cleaned yet and are left out.</p>
  </div>
</Sheet>

<Sheet open={inviteOpen} title="Invite to {info?.name ?? 'the album'}" onclose={() => (inviteOpen = false)} width={460}>
  <div class="stack">
    {#if !invite}
      <div>
        <div class="label">They can</div>
        <Segmented bind:value={role} options={[{ value: 'contributor', label: 'Add photos' }, { value: 'viewer', label: 'Only look' }]} />
      </div>
      <p class="muted small">The invitation works once and for 7 days. You both need to be online when they join.</p>
      <Button kind="primary" block onclick={makeInvite}>Create invitation</Button>
    {:else}
      <div class="qrbox"><QR svg={invite.qr_svg} size={300} /></div>
      <textarea class="mono code" readonly rows="3" value={invite.text} onclick={(e) => (e.currentTarget as HTMLTextAreaElement).select()}></textarea>
      <Button block onclick={copy}>Copy the invitation</Button>
      <p class="muted small">Valid until {dateTime(invite.expires_at)}. Anyone who has it can join, so send it only to the person you mean.</p>
    {/if}
  </div>
</Sheet>

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
  .head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .head h2 { margin: 0; font-size: 20px; }
  .back { text-decoration: none; color: var(--accent); font-size: 14px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 2px; }
  .tile, .ptile { position: relative; aspect-ratio: 1; border: 0; padding: 0; background: var(--surface-2); cursor: pointer; overflow: hidden; }
  .tile img, .ptile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .ph { position: absolute; inset: 0; background: linear-gradient(135deg, var(--surface-2), var(--surface-3)); }
  .vid { position: absolute; left: 5px; bottom: 4px; font-size: 10px; color: #fff; text-shadow: 0 1px 2px rgba(0,0,0,.6); }
  .check { position: absolute; right: 5px; top: 5px; width: 18px; height: 18px; border-radius: 50%; border: 2px solid #fff; background: rgba(0,0,0,.15); color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .sel .check { background: var(--accent); }
  .sel::after { content: ''; position: absolute; inset: 0; background: rgba(255,255,255,.25); pointer-events: none; }
  .more { text-align: center; padding: 10px; }
  .selbar { display: flex; align-items: center; gap: 10px; }
  .empty { margin-top: 24px; }
  .list { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 10px; padding: 10px 2px; border-top: 1px solid var(--line); }
  .pick { display: grid; grid-template-columns: repeat(auto-fill, minmax(84px, 1fr)); gap: 3px; max-height: 50vh; overflow: auto; }
  .qrbox { display: grid; place-items: center; }
  .code { width: 100%; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px; color: var(--ink); font-size: 11px; }
  .viewer { position: fixed; inset: 0; z-index: 30; background: #0b0b0c; display: flex; flex-direction: column; padding-top: var(--safe-top); color: #fff; }
  .vbar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; }
  .vb { background: rgba(255,255,255,.12); color: #fff; border: 0; border-radius: 10px; padding: 7px 12px; font: inherit; font-size: 13px; cursor: pointer; }
  .vb:disabled { opacity: .5; }
  .stage { position: relative; flex: 1; min-height: 0; display: grid; place-items: center; gap: 12px; padding-bottom: var(--safe-bottom); }
  .stage img, .stage video { max-width: 100%; max-height: 100%; object-fit: contain; }
  .nav { position: absolute; top: 50%; transform: translateY(-50%); background: rgba(255,255,255,.14); color: #fff; border: 0; width: 40px; height: 56px; font-size: 28px; border-radius: 10px; cursor: pointer; }
  .nav.l { left: 8px; } .nav.r { right: 8px; }
  .label { font-size: 12px; font-weight: 600; color: var(--ink-3); margin-bottom: 6px; }
</style>
