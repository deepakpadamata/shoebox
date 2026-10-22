<script lang="ts">
  import { blobUrl } from '../lib/api';
  import { ensurePoster } from '../lib/poster';
  import { dateTime } from '../lib/format';
  interface Props {
    id: string; size: number; favorite?: boolean; video?: boolean; copies?: number; target?: number;
    transferring?: boolean; selected?: boolean; selecting?: boolean; hasThumb?: boolean;
    hasPoster?: boolean; hasOriginal?: boolean; index?: number; takenAt?: number;
    onclick?: (e: MouseEvent) => void;
  }
  let { id, size, favorite = false, video = false, copies, target = 2, transferring = false, selected = false, selecting = false, hasThumb = true, hasPoster = false, hasOriginal = false, index, takenAt, onclick }: Props = $props();
  // A video's poster is made on this device once the original is here; until then a neutral tile.
  let madePoster = $state(false);
  $effect(() => {
    if (video && !hasPoster && hasOriginal) ensurePoster(id).then((ok) => (madePoster = ok));
  });
  const poster = $derived(video && (hasPoster || madePoster));
  // Badges only where it matters (design README): red = not backed up enough, blue = in transit.
  const label = $derived(`${video ? 'Video' : 'Photo'}${takenAt ? ` from ${dateTime(takenAt)}` : ''}${favorite ? ', favorite' : ''}${copies !== undefined && copies < target ? `, only ${copies} ${copies === 1 ? 'copy' : 'copies'}` : ''}`);
  const risk = $derived(copies !== undefined && copies < target);
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
  {#if selecting}<span class="sel" aria-hidden="true">{selected ? '✓' : ''}</span>{/if}
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
  .sel { position: absolute; right: 5px; top: 5px; width: 18px; height: 18px; border-radius: 50%; border: 2px solid #fff; background: rgba(0,0,0,.15); color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .selected .sel { background: var(--accent); }
  .selected::after { content: ''; position: absolute; inset: 0; background: rgba(255,255,255,.25); pointer-events: none; }
</style>
