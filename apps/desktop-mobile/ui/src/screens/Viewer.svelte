<script lang="ts">
  import Icon from '../components/Icon.svelte';
  // Full-size viewer (M3-08/09): the preview shows at once, the original replaces it when it is
  // here or once fetched on demand; the info panel says where every copy is stored.
  import { onMount, untrack } from 'svelte';
  import Sheet from '../components/Sheet.svelte';
  import { api, blobUrl, motionSrc, playsNatively, videoUrl, type FaceOut, type PhotoDetail, type PhotoSummary, type SharedItemInfo } from '../lib/api';
  import FaceCrop from '../components/FaceCrop.svelte';
  import { bytes, dateTime, plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';
  import Pill from '../components/Pill.svelte';
  import Row from '../components/Row.svelte';
  import List from '../components/List.svelte';

  /** A photo of a shared album (U1-20): the same viewer, with the album's blob scope. */
  export interface SharedContext {
    album: string;
    /** `owner`, `contributor` or `viewer`. */
    role: string;
    /** Who added what; the viewer shows the attribution and what the original needs. */
    meta: SharedItemInfo[];
    /** Save to the person's own library. */
    onsave: (id: string) => void;
    /** An original arrived: reload the album. */
    onchange: () => void;
  }
  interface Props { items: PhotoSummary[]; index: number; onclose: () => void; onmove: (delta: number) => void; onsimilar?: (id: string) => void; shared?: SharedContext; }
  let { items, index, onclose, onmove, onsimilar, shared }: Props = $props();
  const album = $derived(shared?.album);
  const smeta = $derived(shared?.meta.find((m) => m.id === photo.id));

  // The viewer takes one history entry, so the system back gesture (Android) closes it and
  // returns to the library instead of leaving the app. Closing it any other way gives the
  // entry back.
  onMount(() => {
    const mine = { viewer: Math.random() };
    history.pushState(mine, '');
    // A sheet opened over the viewer owns the entry above this one; its Back must not close the viewer.
    const onpop = () => { if (history.state?.viewer !== mine.viewer) onclose(); };
    addEventListener('popstate', onpop);
    return () => {
      removeEventListener('popstate', onpop);
      if (history.state?.viewer === mine.viewer) history.back();
    };
  });

  // On a phone the info panel is a bottom sheet over the photo: it takes a history entry like a
  // sheet (system Back closes it first), and a tap outside it closes it.
  $effect(() => {
    if (!(info && viewport.mobile)) return;
    const mine = Math.random();
    history.pushState({ sheet: mine }, '');
    const onpop = () => { if (history.state?.sheet !== mine) info = false; };
    addEventListener('popstate', onpop);
    return () => {
      removeEventListener('popstate', onpop);
      if (history.state?.sheet === mine) history.back();
    };
  });

  const joinNames = (n: string[]) => (n.length === 0 ? 'no device' : n.length === 1 ? n[0] : `${n.slice(0, -1).join(', ')} and ${n[n.length - 1]}`);
  const photo = $derived(items[index]);
  let detail = $state<PhotoDetail | null>(null);
  import { viewport } from '../lib/viewport.svelte';
  import { pinch, swipe } from '../lib/gestures';
  let info = $state(false);
  let fetching = $state(false);
  let zoom = $state(1);
  let pullY = $state(0);
  let originalReady = $state(false);
  // A Live Photo or motion photo: its video plays over the picture (F2.10).
  let playingLive = $state(false);
  let liveFetching = $state(false);
  let liveSrc = $state<string | null>(null);
  // A video opening in the platform's player.
  let starting = $state(false);
  // The original is here but the media element could not play it (a codec this device lacks).
  let unplayable = $state(false);

  $effect(() => {
    const id = photo.id;
    // Only the photo is a dependency. `stopLive` reads `liveSrc`: tracked, setting it to play a
    // Live Photo would run this again and stop the video the moment it started.
    untrack(() => {
      detail = null;
      originalReady = false;
      stopLive();
      liveFetching = false;
      starting = false;
      unplayable = false;
      fetching = false;
      zoom = 1;
      pullY = 0;
    });
    faces = [];
    if (album) {
      originalReady = !!smeta?.has_original;
      return;
    }
    api.photoDetail(id).then((d) => {
      if (photo.id === id) {
        detail = d;
        originalReady = d.has_original;
      }
    }).catch(fail);
    api.photoFaces(id).then((f) => { if (photo.id === id) faces = f; }).catch(() => {});
  });

  // Phones open with the info closed; computers with whatever the person left it at.
  info = viewport.mobile ? false : (() => { try { return (localStorage.getItem('viewerInfo') ?? '1') === '1'; } catch { return true; } })();

  const focusNow = (el: HTMLElement) => el.focus();
  // People in this photo (S1-19): name a face, or say who it is not.
  let faces = $state<FaceOut[]>([]);
  let naming = $state<number | null>(null);
  let faceName = $state('');
  async function nameFace(f: FaceOut) {
    try {
      await api.labelFace(f.photo, f.rect, { name: faceName });
      naming = null;
      faceName = '';
      faces = await api.photoFaces(f.photo);
    } catch (e) { fail(e); }
  }
  async function notThem(f: FaceOut) {
    try {
      await api.labelFace(f.photo, f.rect, { not: f.person! });
      faces = await api.photoFaces(f.photo);
      toast(`Not ${f.name}`);
    } catch (e) { fail(e); }
  }

  // A shared album's original may arrive while the photo is open.
  $effect(() => { if (album && smeta?.has_original) originalReady = true; });

  // A video's preview and thumbnail are the video itself (no poster frames in the record yet):
  // an image of one only fails, slowly. Show this device's poster frame, or nothing.
  const poster = $derived(photo.is_video && photo.has_poster ? blobUrl(photo.id, 'poster', album) : null);
  // The web view shows these as they are; HEIC, AVIF, TIFF, BMP and RAW originals are kept, and the preview is what is shown.
  const webFormat = $derived(album ? !photo.is_video : ['image/jpeg', 'image/png', 'image/gif', 'image/webp'].includes(detail?.mime ?? ''));
  const src = $derived(photo.is_video ? poster : originalReady && webFormat ? blobUrl(photo.id, 'original', album) : blobUrl(photo.id, album ? 'preview' : detail?.has_preview === false ? 'thumb' : 'preview', album));

  // Only the element showing the original can say the original is unreadable: a preview that
  // fails to load (or one still loading from before the original was ready) says nothing.
  function imageFailed() {
    if (!photo.is_video && originalReady && webFormat) originalFailed();
  }

  // Play the video of a Live Photo, getting it from another device first if this one has not got it.
  async function playLive() {
    if (playingLive) { stopLive(); return; }
    if (!detail?.has_motion) {
      liveFetching = true;
      const id = photo.id;
      try {
        await api.fetchNow(id, 'motion');
        const d = await api.photoDetail(id);
        if (photo.id !== id) return;
        detail = d;
      } catch (e) {
        fail(e);
        return;
      } finally {
        liveFetching = false;
      }
    }
    const id = photo.id;
    try {
      const src = await motionSrc(id);
      if (photo.id !== id) { if (src.startsWith('blob:')) URL.revokeObjectURL(src); return; }
      liveSrc = src;
      playingLive = true;
    } catch (e) {
      fail(e);
    }
  }

  function stopLive() {
    playingLive = false;
    if (liveSrc?.startsWith('blob:')) URL.revokeObjectURL(liveSrc);
    liveSrc = null;
  }

  // Android: the video opens in the system's player over the app.
  async function playNative() {
    starting = true;
    try {
      await api.playVideo(photo.id, album);
    } catch (e) {
      fail(e);
    } finally {
      starting = false;
    }
  }

  function liveFailed() {
    stopLive();
    toast('This device cannot play the video of this Live Photo');
  }

  async function getOriginal() {
    fetching = true;
    try {
      if (album) {
        await api.sharedFetch(album, photo.id, 'original');
        originalReady = true;
        shared!.onchange();
        return;
      }
      await api.fetchNow(photo.id, 'original');
      detail = await api.photoDetail(photo.id);
      originalReady = detail.has_original;
    } catch (e) {
      fail(e);
    } finally {
      fetching = false;
    }
  }

  // The original this device had could not be read (a gallery file deleted or changed under
  // it): show the preview, and get the original from another device.
  async function originalFailed() {
    if (!originalReady) return;
    const id = photo.id;
    originalReady = false;
    try {
      const d = await api.photoDetail(id);
      if (photo.id !== id) return;
      detail = d;
      if (!d.has_original) await getOriginal();
      else unplayable = true;
    } catch (e) {
      fail(e);
    }
  }

  async function toggleFavorite() {
    await api.setFavorite([photo.id], !photo.favorite).catch(fail);
  }

  async function togglePin() {
    if (!detail) return;
    await api.pinOriginal(photo.id, !detail.pinned).catch(fail);
    detail = await api.photoDetail(photo.id);
    toast(detail.pinned ? 'This original stays on this device' : 'No longer pinned here');
  }

  async function remove() {
    await api.delete([photo.id]).then(() => toast('Moved to Recently Deleted')).catch(fail);
    if (items.length > 1) onmove(index === items.length - 1 ? -1 : 0);
    else onclose();
  }

  // Share (U1-16): the original goes to another app through the system share sheet.
  let shareBtn: HTMLElement | undefined = $state();
  let sharing = $state(false);
  async function share() {
    sharing = true;
    const r = shareBtn?.getBoundingClientRect();
    try {
      await api.shareFiles([photo.id], r ? { x: r.left, y: r.top, w: r.width, h: r.height } : undefined);
    } catch (e) { fail(e); } finally { sharing = false; }
  }

  // The ⋯ menu: the rarer things.
  let menu = $state(false);
  let albumPick = $state(false);
  const myAlbums = $derived(app.albums.filter((a) => a.personal && !a.sharedId));
  async function addToAlbum(id: string) {
    try {
      await api.albumAdd(id, [photo.id]);
      toast('Added to the album');
      albumPick = false;
    } catch (e) { fail(e); }
  }

  // The controls hide when the photo is tapped; the info panel remembers whether it was open.
  let chrome = $state(true);
  const INFO_KEY = 'viewerInfo';
  $effect(() => { try { localStorage.setItem(INFO_KEY, info ? '1' : '0'); } catch { /* storage may be blocked */ } });
  const date = $derived(dateTime(photo.taken_at));

  function key(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 's' && !album && !(e.target instanceof HTMLInputElement)) { share(); return; }
    if (e.key === 'f' && !album && !(e.target instanceof HTMLInputElement)) { toggleFavorite(); return; }
    if (e.key === 'Escape') { if (info && viewport.mobile) info = false; else onclose(); }
    else if (e.key === 'ArrowRight') onmove(1);
    else if (e.key === 'ArrowLeft') onmove(-1);
    else if (e.key === 'i') info = !info;
  }

  const unavailable = $derived(detail && !detail.has_original && detail.stored_on.every((s) => s.is_this_device || !s.online) && detail.stored_on.some((s) => s.kinds.includes('original')));
</script>

<svelte:window onkeydown={key} />

<div class="viewer" role="dialog" aria-modal="true" aria-label="Photo viewer" class:bare={!chrome}>
  <div class="stage" use:swipe={{ enabled: () => zoom === 1, onSwipe: (d) => (d === 'left' ? onmove(1) : d === 'right' ? onmove(-1) : d === 'down' ? (info ? (info = false) : onclose()) : d === 'up' ? (info = true) : null), onDrag: (_, dy) => (pullY = info ? 0 : Math.max(0, dy)), onEnd: () => (pullY = 0) }} use:pinch={{ onScale: (s) => (zoom = Math.min(5, Math.max(1, s))), onEnd: (s) => (zoom = Math.min(5, Math.max(1, s))) }}>
    <div class="bar top">
      <button class="icon" aria-label="Close" title="Close (Esc)" onclick={onclose}><Icon name="back" /></button>
      <span class="when">{date}{#if smeta}<small> · {smeta.mine ? 'you' : smeta.added_by}</small>{/if}</span>
      <span class="grow"></span>
      {#if detail?.live && !photo.is_video}
        <button class="live" aria-pressed={playingLive} aria-label={playingLive ? 'Stop the Live Photo' : 'Play the Live Photo'} disabled={liveFetching} onclick={playLive}><Icon name="live" size={18} /> {liveFetching ? 'Getting video…' : playingLive ? 'Stop' : 'Live'}</button>
      {/if}
      {#if !viewport.mobile}
        <!-- On a computer the actions are in the toolbar, with their shortcuts. -->
        {#if !album}
          <button class="tool" bind:this={shareBtn} title="Share (S)" disabled={sharing} onclick={share}><Icon name="share" size={20} /> Share</button>
          <button class="tool" aria-pressed={photo.favorite} title="Favorite (F)" onclick={toggleFavorite}><Icon name="heart" size={20} fill={photo.favorite} /> Favorite</button>
        {:else}
          <button class="tool" title="Save to my library" onclick={() => shared!.onsave(photo.id)}><Icon name="download" size={20} /> Save to library</button>
        {/if}
        <button class="tool" aria-pressed={info} title="Info (I)" onclick={() => (info = !info)}><Icon name="info" size={20} /> Info</button>
        {#if !album}<button class="tool danger" title="Delete from all devices" onclick={remove}><Icon name="trash" size={20} /> Delete</button>{/if}
      {/if}
      {#if !album}<button class="icon" aria-label="More" title="More" onclick={() => (menu = true)}><Icon name="more" /></button>{/if}
    </div>
    <button class="nav left" aria-label="Previous" disabled={index === 0} onclick={() => onmove(-1)}>‹</button>
    <div class="media" onclick={(e) => { if (!(e.target as HTMLElement).closest('button, video')) chrome = !chrome; }} role="presentation">
      {#if photo.is_video && originalReady && playsNatively()}
        {#if poster}<img src={poster} alt={`Video from ${dateTime(photo.taken_at)}`} />{/if}
        <button class="play" aria-label="Play the video" disabled={starting} onclick={playNative}><Icon name="play" size={34} fill /></button>
      {:else if photo.is_video && originalReady}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={videoUrl(photo.id)} poster={poster ?? undefined} controls autoplay onerror={originalFailed}></video>
      {:else if playingLive}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={liveSrc} autoplay playsinline onended={stopLive} onerror={liveFailed} onclick={stopLive}></video>
      {:else}
        {#key photo.id}{#if src}<img {src} alt={`${photo.is_video ? 'Video' : 'Photo'} from ${dateTime(photo.taken_at)}`} style="transform: translateY({pullY}px) scale({zoom}); opacity: {1 - Math.min(0.5, pullY / 400)}" ondblclick={() => (zoom = zoom > 1 ? 1 : 2.5)} onerror={imageFailed} />{/if}{/key}
      {/if}
      {#if detail && !originalReady}
        <div class="overlay">
          {#if unplayable}
            <b>{photo.is_video ? 'This video cannot be played here' : 'This photo cannot be shown here'}</b><span>The original is on this device, but this device cannot open its format. The preview is what is shown.</span>
          {:else if unavailable}
            <b>Original unavailable</b><span>The devices that have it are offline. The preview is what is here.</span>
          {:else}
            <Button kind="primary" size="sm" disabled={fetching} onclick={getOriginal}>{fetching ? 'Fetching original…' : `Get original (${bytes(detail.size)})`}</Button>
          {/if}
        </div>
      {/if}
    </div>
    <button class="nav right" aria-label="Next" disabled={index >= items.length - 1} onclick={() => onmove(1)}>›</button>
  </div>


  {#if viewport.mobile && chrome}
    <div class="bar bottom">
      {#if !album}
        <button class="act" bind:this={shareBtn} disabled={sharing} onclick={share}><Icon name="share" /><span>Share</span></button>
        <button class="act" aria-pressed={photo.favorite} onclick={toggleFavorite}><Icon name="heart" fill={photo.favorite} /><span>Favorite</span></button>
      {:else}
        <button class="act" onclick={() => shared!.onsave(photo.id)}><Icon name="download" /><span>Save</span></button>
      {/if}
      <button class="act" aria-pressed={info} onclick={() => (info = !info)}><Icon name="info" /><span>Info</span></button>
      {#if !album}<button class="act danger" onclick={remove}><Icon name="trash" /><span>Delete</span></button>{/if}
    </div>
  {/if}

  {#if info && viewport.mobile}
    <div class="infoscrim" role="presentation" onclick={() => (info = false)}></div>
  {/if}
  {#if info}
    <aside class="info">
      {#if faces.length}
        <div class="people">
          <span class="small muted">People in this photo</span>
          {#each faces as f, i}
            <div class="face-row">
              <FaceCrop face={f} size={36} />
              {#if naming === i}
                <input class="input grow" placeholder="Name" bind:value={faceName} use:focusNow onkeydown={(e) => e.key === 'Enter' && nameFace(f)} />
                <Button size="sm" disabled={!faceName.trim()} onclick={() => nameFace(f)}>Save</Button>
              {:else if f.person}
                <span class="grow small">{f.name || 'Unnamed'}</span>
                <button class="link small" onclick={() => notThem(f)}>Not {f.name || 'them'}</button>
              {:else}
                <span class="grow small muted">Someone</span>
                <button class="link small" onclick={() => { naming = i; faceName = ''; }}>Name</button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if album}
        <div class="stack">
          <div>
            <b>{date}</b>
            {#if smeta}<div class="muted small">Added by {smeta.mine ? 'you' : smeta.added_by}{smeta.location_removed ? ' · location removed' : ''}</div>{/if}
          </div>
          <div class="muted small">{smeta?.has_original ? 'The original is on this device.' : 'Only the preview is on this device. Get the original to see it in full.'}</div>
        </div>
      {:else if detail}
        <div class="stack">
          <div>
            <b>{dateTime(detail.taken_at)}</b>
            <div class="muted small">{detail.width} × {detail.height} · {bytes(detail.size)} · {detail.mime.replace('image/', '').replace('video/', '').toUpperCase()}</div>
            {#if detail.camera}<div class="muted small">{detail.camera}{detail.lens ? ` · ${detail.lens}` : ''}</div>{/if}
            {#if detail.gps}<div class="muted small"><Icon name="place" size={14} /> {detail.gps[0].toFixed(4)}, {detail.gps[1].toFixed(4)}</div>{/if}
          </div>
          <div class="row-flex">
            {#if detail.backed_up}
              <Pill tone="ok">Backed up · on {joinNames(detail.held_by)}</Pill>
            {:else}
              <Pill tone="risk">Not backed up yet · only on {joinNames(detail.held_by)}</Pill>
            {/if}
            {#if detail.pinned}<Pill tone="info"><Icon name="pin" size={14} /> Kept here</Pill>{/if}
          </div>
          <div class="section">Stored on</div>
          <List>
            {#each detail.stored_on as s}
              <Row title={s.is_this_device ? `${s.name} (this device)` : s.name} subtitle={s.kinds.map((k) => (k === 'original' ? 'original' : k)).join(' · ')}>
                {#snippet trail()}<span class="dot" class:on={s.online} title={s.online ? 'online' : 'offline'}></span>{/snippet}
              </Row>
            {/each}
          </List>
          {#if app.state?.role !== 'keeper'}<Button block onclick={togglePin}>{detail.pinned ? 'Stop keeping on this device' : 'Keep original on this device'}</Button>{/if}
        </div>
      {:else}
        <span class="muted">Loading…</span>
      {/if}
    </aside>
  {/if}
</div>

<Sheet open={menu} title="More" onclose={() => (menu = false)} width={340}>
  <div class="menu">
    <button onclick={() => { menu = false; albumPick = true; }}><Icon name="addalbum" size={20} /> Add to album</button>
    {#if onsimilar}<button onclick={() => { menu = false; onsimilar(photo.id); }}><Icon name="search" size={20} /> Find similar</button>{/if}
    {#if detail && app.state?.role !== 'keeper'}<button onclick={() => { menu = false; togglePin(); }}><Icon name="pin" size={20} /> {detail.pinned ? 'Stop keeping on this phone' : 'Keep on this phone'}</button>{/if}
  </div>
</Sheet>

<Sheet open={albumPick} title="Add to album" onclose={() => (albumPick = false)} width={340}>
  <div class="menu">
    {#each myAlbums as a (a.id)}<button onclick={() => addToAlbum(a.id)}><Icon name="albums" size={20} /> {a.name}</button>{:else}<p class="muted small">You have no albums yet. Create one in Albums.</p>{/each}
  </div>
</Sheet>

<style>
  .viewer { position: fixed; inset: 0; z-index: 30; background: #0b0a09; color: #fff; display: flex; }
  .stage { position: relative; flex: 1; min-width: 0; display: flex; align-items: center; justify-content: center; }
  .stage { touch-action: pan-y pinch-zoom; }
  img { transition: transform .08s; touch-action: none; }
  .media { width: 100%; height: 100%; min-height: 0; display: grid; place-items: center; padding: 56px 56px; position: relative; }
  img, video { max-width: 100%; max-height: calc(100vh - 112px); object-fit: contain; border-radius: 4px; }
  .bar { position: absolute; left: 0; right: 0; top: 0; display: flex; gap: 6px; padding: 10px 14px; z-index: 2; }
  .icon { background: rgba(255,255,255,.1); border: 0; width: 40px; height: 40px; display: grid; place-items: center; border-radius: 50%; cursor: pointer; font-size: 16px; color: #fff; }
  .nav { position: absolute; top: 50%; transform: translateY(-50%); z-index: 2; background: rgba(255,255,255,.1); border: 0; color: #fff; width: 40px; height: 64px; font-size: 28px; cursor: pointer; border-radius: 10px; }
  .nav:disabled { opacity: .25; cursor: default; }
  .left { left: 10px; } .right { right: 10px; }
  .live { display: inline-flex; align-items: center; gap: 6px; align-self: center; height: 36px; background: rgba(255,255,255,.1); color: #fff; border: 0; border-radius: 999px; padding: 0 14px; font: inherit; font-size: var(--t-sm); font-weight: 600; cursor: pointer; }
  .live[aria-pressed='true'] { background: var(--accent); color: var(--accent-ink); }
  .live:disabled { opacity: .7; cursor: default; }
  .play { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); z-index: 2; width: 72px; height: 72px; border-radius: 50%; border: 0; background: rgba(0,0,0,.55); color: #fff; font-size: 28px; padding-left: 6px; cursor: pointer; }
  .play:disabled { opacity: .6; }
  .overlay { position: absolute; bottom: 24px; left: 50%; transform: translateX(-50%); background: rgba(0,0,0,.7); padding: 10px 14px; border-radius: 12px; display: flex; flex-direction: column; gap: 4px; align-items: center; font-size: 13px; }
  .when { align-self: center; font-weight: 600; font-size: var(--t-md); text-shadow: 0 1px 2px rgba(0,0,0,.5); }
  .when small { font-weight: 400; opacity: .8; }
  .tool { display: inline-flex; align-items: center; gap: 6px; background: rgba(255,255,255,.1); border: 0; color: #fff; border-radius: 999px; padding: 0 14px; height: 36px; font: inherit; font-size: var(--t-sm); cursor: pointer; }
  .tool[aria-pressed='true'] { background: var(--accent); color: var(--accent-ink); }
  .tool.danger { color: #ff9b8c; }
  .tool:disabled { opacity: .6; }
  .bar.bottom { top: auto; bottom: 0; justify-content: space-around; padding: 6px 8px calc(6px + var(--safe-bottom)); background: linear-gradient(transparent, rgba(0,0,0,.65)); }
  .act { display: grid; justify-items: center; gap: 2px; min-width: var(--tap); min-height: var(--tap); background: none; border: 0; color: #fff; font: inherit; font-size: var(--t-xs); cursor: pointer; }
  .act[aria-pressed='true'] { color: var(--accent); }
  .act.danger { color: #ff9b8c; }
  .act:disabled { opacity: .6; }
  .bare .bar, .bare .nav { opacity: 0; pointer-events: none; }
  .bar, .nav { transition: opacity .15s; }
  .menu { display: grid; }
  .menu button { display: flex; align-items: center; gap: 12px; text-align: left; border: 0; border-top: 1px solid var(--line); background: none; color: var(--ink); font: inherit; min-height: var(--tap); cursor: pointer; }
  .menu button:first-child { border-top: 0; }
  .info { width: 320px; flex: none; background: var(--surface); color: var(--ink); padding: 18px 16px; overflow: auto; border-left: 1px solid var(--line); }
  @media (max-width: 720px) {
    .viewer { flex-direction: column; padding-top: var(--safe-top); }
    .infoscrim { position: fixed; inset: 0; z-index: 3; background: rgba(0,0,0,.25); }
    .info { position: fixed; left: 0; right: 0; bottom: 0; z-index: 4; width: auto; max-height: 60vh; border-left: 0; border-top: 1px solid var(--line); border-radius: 18px 18px 0 0; padding-bottom: calc(18px + var(--safe-bottom)); box-shadow: var(--shadow); }
    .media { padding: 52px 4px 8px; }
    img, video { max-height: calc(100vh - 170px); }
    .nav { display: none; }
  }
  .section { font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin-top: 14px !important; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--ink-3); display: inline-block; }
  .dot.on { background: var(--ok); }
  .people { display: flex; flex-direction: column; gap: 6px; margin-bottom: 12px; }
  .face-row { display: flex; align-items: center; gap: 8px; }
  .face-row .grow { flex: 1; min-width: 0; }
  .link { border: 0; background: none; color: var(--accent); cursor: pointer; padding: 0; }
</style>
