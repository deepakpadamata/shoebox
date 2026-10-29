<script lang="ts">
  import { blobUrl } from '../lib/api';
  import { ensurePoster } from '../lib/poster';
  import { dateTime } from '../lib/format';
  interface Props {
    id: string; size: number; favorite?: boolean; video?: boolean; copies?: number; target?: number;
    transferring?: boolean; selected?: boolean; selecting?: boolean; hasThumb?: boolean;
    hasPoster?: boolean; needsPoster?: boolean; hasOriginal?: boolean; backedUp?: boolean; index?: number; takenAt?: number;
    onclick?: (e: MouseEvent) => void;
  }
  let { id, size, favorite = false, video = false, copies, target = 2, transferring = false, selected = false, selecting = false, hasThumb = true, hasPoster = false, needsPoster = false, hasOriginal = false, backedUp = true, index, takenAt, onclick }: Props = $props();
  // A video nobody has made a poster for yet gets one here if this device has the original;
  // until then a neutral tile. (One made elsewhere arrives like any thumbnail.)
  let madePoster = $state(false);
  $effect(() => {
    if (!(video && needsPoster && hasOriginal)) return;
    const { result, release } = ensurePoster(id);
    result.then((ok) => (madePoster = ok));
    return release;
  });
  const poster = $derived(video && (hasPoster || madePoster));
  // Badges only where it matters (design README): red = not backed up enough, blue = in transit.
  const label = $derived(`${video ? 'Video' : 'Photo'}${takenAt ? ` from ${dateTime(takenAt)}` : ''}${favorite ? ', favorite' : ''}${copies !== undefined && copies < target && !backedUp ? `, only ${copies} ${copies === 1 ? 'copy' : 'copies'}` : ''}`);
  const risk = $derived(copies !== undefined && copies < target && !backedUp);
</script>

<button class="tile" class:selected data-index={index} style="width:{size}px;height:{size}px" {onclick} aria-label={label} aria-pressed={selecting ? selected : undefined}>
  {#if poster}
    <img src={blobUrl(id, 'poster')} alt="" loading="lazy" decoding="async" draggable="false" />
  {:else if hasThumb && !video}
    <img src={blobUrl(id, 'thumb')} alt="" loading="lazy" decoding="async" draggable="false" />
  {:else}
    <span class="placeholder" aria-hidden="true"></span>
  {/if}
  {#if favorite}<span class="fav" aria-label="Favorite">♥</span>{/if}
  {#if video}<span class="vid" aria-label="Video">▶</span>{/if}
  {#if transferring}<span class="b sync" title="Transferring">↻</span>
  {:else if risk}<span class="b risk" title="Only {copies} {copies === 1 ? 'copy' : 'copies'}">{copies} {copies === 1 ? 'copy' : 'copies'}</span>{/if}
  <span class="sel" class:on={selecting} aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M7 12.5l3.2 3.2L17 9" /></svg></span>
</button>

<style>
  .tile { position: relative; overflow: hidden; padding: 0; border: 0; background: var(--surface-2); cursor: pointer; display: block; }
  img { width: 100%; height: 100%; object-fit: cover; display: block; user-select: none; }
  .placeholder { position: absolute; inset: 0; background: linear-gradient(135deg, var(--surface-2), var(--surface-3)); }
  /* Videos have no poster frame yet (M3-10): a neutral tile with the play mark. */
  .b { position: absolute; right: 4px; bottom: 4px; font-size: 9px; font-weight: 700; padding: 2px 5px; border-radius: 6px; background: rgba(0,0,0,.55); color: #fff; }
  .b.risk { background: var(--risk); color: #fff; }
  .b.sync { background: var(--info); color: #fff; }
  .vid { position: absolute; left: 5px; bottom: 4px; font-size: 10px; color: #fff; font-weight: 600; text-shadow: 0 1px 2px rgba(0,0,0,.6); }
  .fav { position: absolute; left: 5px; top: 3px; font-size: 11px; color: #fff; text-shadow: 0 1px 2px rgba(0,0,0,.6); }
  /* The select circle: on every tile while selecting, and on the one under the mouse. */
  .sel { position: absolute; right: 4px; top: 4px; width: 26px; height: 26px; border-radius: 50%; display: grid; place-items: center;
    opacity: 0; transition: opacity 0.12s; cursor: pointer; }
  .sel::before { content: ''; position: absolute; inset: 4px; border-radius: 50%; border: 2px solid #fff; background: rgba(0,0,0,.2); box-shadow: 0 1px 3px rgba(0,0,0,.35); }
  .sel svg { position: relative; width: 14px; height: 14px; fill: none; stroke: #fff; stroke-width: 3; stroke-linecap: round; stroke-linejoin: round; opacity: 0; }
  .sel.on, .selected .sel { opacity: 1; }
  .selected .sel::before { background: var(--accent); border-color: var(--accent); }
  .selected .sel svg { opacity: 1; }
  @media (hover: hover) {
    .tile:hover .sel { opacity: 1; }
    .tile:hover .sel:not(.on) svg { opacity: 0.7; }
    .tile:not(.selected) .sel:hover::before { background: rgba(0,0,0,.4); }
  }
  /* Without a mouse a hidden circle takes no taps: the tap opens the photo. */
  @media (hover: none) { .sel:not(.on) { pointer-events: none; } }
  .selected::after { content: ''; position: absolute; inset: 0; background: rgba(255,255,255,.25); pointer-events: none; }
</style>
