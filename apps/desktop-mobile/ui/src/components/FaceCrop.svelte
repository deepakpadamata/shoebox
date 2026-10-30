<script lang="ts">
  // A face cut out of its photo's preview (S1-19): a square around the face box, from the
  // photo's size, so no image is cut on the device beforehand.
  import { blobUrl, type FaceOut } from '../lib/api';

  let { face, size = 96, round = true }: { face: FaceOut; size?: number; round?: boolean } = $props();
  const style = $derived.by(() => {
    const [x1, y1, x2, y2] = face.rect;
    const w = face.width || 1;
    const h = face.height || 1;
    // The square side in photo pixels: the face box with some room around it.
    const side = Math.max((x2 - x1) * w, (y2 - y1) * h) * 1.5;
    const cx = ((x1 + x2) / 2) * w;
    const cy = ((y1 + y2) / 2) * h;
    const scale = size / side;
    return `width:${w * scale}px;height:${h * scale}px;left:${(side / 2 - cx) * scale}px;top:${(side / 2 - cy) * scale}px`;
  });
</script>

<span class="crop" class:round style="width:{size}px;height:{size}px">
  <img src={blobUrl(face.photo, 'preview')} alt="" style={style} loading="lazy" />
</span>

<style>
  .crop { position: relative; display: inline-block; overflow: hidden; background: var(--surface-2); flex: none; }
  .crop.round { border-radius: 50%; }
  img { position: absolute; max-width: none; }
</style>
