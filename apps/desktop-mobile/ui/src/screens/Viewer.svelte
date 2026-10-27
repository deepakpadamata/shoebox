<script lang="ts">
  // Full-size viewer (M3-08/09): the preview shows at once, the original replaces it when it is
  // here or once fetched on demand; the info panel says where every copy is stored.
  import { onMount } from 'svelte';
  import { api, blobUrl, videoUrl, type PhotoDetail, type PhotoSummary } from '../lib/api';
  import { bytes, dateTime, plural } from '../lib/format';
  import { fail, toast } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';
  import Pill from '../components/Pill.svelte';
  import Row from '../components/Row.svelte';
  import List from '../components/List.svelte';

  interface Props { items: PhotoSummary[]; index: number; onclose: () => void; onmove: (delta: number) => void; }
  let { items, index, onclose, onmove }: Props = $props();

  // The viewer takes one history entry, so the system back gesture (Android) closes it and
  // returns to the library instead of leaving the app. Closing it any other way gives the
  // entry back.
  onMount(() => {
    const mine = { viewer: Math.random() };
    history.pushState(mine, '');
    const onpop = () => onclose();
    addEventListener('popstate', onpop);
    return () => {
      removeEventListener('popstate', onpop);
      if (history.state?.viewer === mine.viewer) history.back();
    };
  });

  const photo = $derived(items[index]);
  let detail = $state<PhotoDetail | null>(null);
  import { viewport } from '../lib/viewport.svelte';
  import { pinch, swipe } from '../lib/gestures';
  let info = $state(!viewport.mobile);
  let fetching = $state(false);
  let zoom = $state(1);
  let pullY = $state(0);
  let originalReady = $state(false);
  // The original is here but the media element could not play it (a codec this device lacks).
  let unplayable = $state(false);

  $effect(() => {
    const id = photo.id;
    detail = null;
    originalReady = false;
    unplayable = false;
    fetching = false;
    zoom = 1;
    pullY = 0;
    api.photoDetail(id).then((d) => {
      if (photo.id === id) {
        detail = d;
        originalReady = d.has_original;
      }
    }).catch(fail);
  });

  // A video's preview and thumbnail are the video itself (no poster frames in the record yet):
  // an image of one only fails, slowly. Show this device's poster frame, or nothing.
  const poster = $derived(photo.is_video && photo.has_poster ? blobUrl(photo.id, 'poster') : null);
  const src = $derived(photo.is_video ? poster : originalReady ? blobUrl(photo.id, 'original') : blobUrl(photo.id, detail?.has_preview === false ? 'thumb' : 'preview'));

  // Only the element showing the original can say the original is unreadable: a preview that
  // fails to load (or one still loading from before the original was ready) says nothing.
  function imageFailed() {
    if (!photo.is_video && originalReady) originalFailed();
  }

  async function getOriginal() {
    fetching = true;
    try {
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

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
    else if (e.key === 'ArrowRight') onmove(1);
    else if (e.key === 'ArrowLeft') onmove(-1);
    else if (e.key === 'i') info = !info;
  }

  const unavailable = $derived(detail && !detail.has_original && detail.stored_on.every((s) => s.is_this_device || !s.online) && detail.stored_on.some((s) => s.kinds.includes('original')));
</script>

<svelte:window onkeydown={key} />

<div class="viewer" role="dialog" aria-modal="true" aria-label="Photo viewer">
  <div class="stage" use:swipe={{ enabled: () => zoom === 1, onSwipe: (d) => (d === 'left' ? onmove(1) : d === 'right' ? onmove(-1) : d === 'down' ? onclose() : null), onDrag: (_, dy) => (pullY = Math.max(0, dy)), onEnd: () => (pullY = 0) }} use:pinch={{ onScale: (s) => (zoom = Math.min(5, Math.max(1, s))), onEnd: (s) => (zoom = Math.min(5, Math.max(1, s))) }}>
    <div class="bar top">
      <button class="icon" aria-label="Close" onclick={onclose}>✕</button>
      <span class="grow"></span>
      <button class="icon" aria-label="Favorite" aria-pressed={photo.favorite} onclick={toggleFavorite}>{photo.favorite ? '♥' : '♡'}</button>
      <button class="icon" aria-label="Info" aria-pressed={info} onclick={() => (info = !info)}>ⓘ</button>
      <button class="icon" aria-label="Delete" onclick={remove}>🗑</button>
    </div>
    <button class="nav left" aria-label="Previous" disabled={index === 0} onclick={() => onmove(-1)}>‹</button>
    <div class="media">
      {#if photo.is_video && originalReady}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={videoUrl(photo.id)} poster={poster ?? undefined} controls autoplay onerror={originalFailed}></video>
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

  {#if info}
    <aside class="info">
      {#if detail}
        <div class="stack">
          <div>
            <b>{dateTime(detail.taken_at)}</b>
            <div class="muted small">{detail.width} × {detail.height} · {bytes(detail.size)} · {detail.mime.replace('image/', '').replace('video/', '').toUpperCase()}</div>
            {#if detail.camera}<div class="muted small">{detail.camera}{detail.lens ? ` · ${detail.lens}` : ''}</div>{/if}
            {#if detail.gps}<div class="muted small">📍 {detail.gps[0].toFixed(4)}, {detail.gps[1].toFixed(4)}</div>{/if}
          </div>
          <div class="row-flex">
            {#if detail.copies < detail.redundancy_target}
              <Pill tone="risk">{plural(detail.copies, 'copy', 'copies')} · wants {detail.redundancy_target}</Pill>
            {:else}
              <Pill tone="ok">{plural(detail.copies, 'copy', 'copies')}</Pill>
            {/if}
            {#if detail.pinned}<Pill tone="info">📌 Kept here</Pill>{/if}
          </div>
          <div class="section">Stored on</div>
          <List>
            {#each detail.stored_on as s}
              <Row title={s.is_this_device ? `${s.name} (this device)` : s.name} subtitle={s.kinds.map((k) => (k === 'original' ? 'original' : k)).join(' · ')}>
                {#snippet trail()}<span class="dot" class:on={s.online} title={s.online ? 'online' : 'offline'}></span>{/snippet}
              </Row>
            {/each}
          </List>
          <Button block onclick={togglePin}>{detail.pinned ? 'Stop keeping on this device' : '📌 Keep original on this device'}</Button>
        </div>
      {:else}
        <span class="muted">Loading…</span>
      {/if}
    </aside>
  {/if}
</div>

<style>
  .viewer { position: fixed; inset: 0; z-index: 30; background: #0b0a09; color: #fff; display: flex; }
  .stage { position: relative; flex: 1; min-width: 0; display: flex; align-items: center; justify-content: center; }
  .stage { touch-action: pan-y pinch-zoom; }
  img { transition: transform .08s; touch-action: none; }
  .media { width: 100%; height: 100%; min-height: 0; display: grid; place-items: center; padding: 56px 56px; position: relative; }
  img, video { max-width: 100%; max-height: calc(100vh - 112px); object-fit: contain; border-radius: 4px; }
  .bar { position: absolute; left: 0; right: 0; top: 0; display: flex; gap: 6px; padding: 10px 14px; z-index: 2; }
  .icon { background: rgba(255,255,255,.1); border: 0; width: 36px; height: 36px; border-radius: 50%; cursor: pointer; font-size: 16px; color: #fff; }
  .icon[aria-pressed='true'] { background: var(--accent); color: var(--accent-ink); }
  .nav { position: absolute; top: 50%; transform: translateY(-50%); z-index: 2; background: rgba(255,255,255,.1); border: 0; color: #fff; width: 40px; height: 64px; font-size: 28px; cursor: pointer; border-radius: 10px; }
  .nav:disabled { opacity: .25; cursor: default; }
  .left { left: 10px; } .right { right: 10px; }
  .overlay { position: absolute; bottom: 24px; left: 50%; transform: translateX(-50%); background: rgba(0,0,0,.7); padding: 10px 14px; border-radius: 12px; display: flex; flex-direction: column; gap: 4px; align-items: center; font-size: 13px; }
  .info { width: 320px; flex: none; background: var(--surface); color: var(--ink); padding: 18px 16px; overflow: auto; border-left: 1px solid var(--line); }
  @media (max-width: 720px) {
    .viewer { flex-direction: column; padding-top: var(--safe-top); }
    .info { width: auto; max-height: 48vh; border-left: 0; border-top: 1px solid var(--line); border-radius: 18px 18px 0 0; padding-bottom: calc(18px + var(--safe-bottom)); }
    .media { padding: 52px 4px 8px; }
    img, video { max-height: calc(100vh - 140px); }
    .nav { display: none; }
  }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin-top: 14px !important; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--ink-3); display: inline-block; }
  .dot.on { background: var(--ok); }
</style>
