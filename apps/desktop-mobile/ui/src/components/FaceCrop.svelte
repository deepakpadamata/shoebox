<script lang="ts">
  // A face cut out of its photo (S1-19): the app cuts a small square around the face box and
  // keeps it, so a screen of faces never loads whole photos (on a phone often the originals).
  import { blobUrl, type BlobKind, type FaceOut } from '../lib/api';

  let { face, size = 96, round = true }: { face: FaceOut; size?: number; round?: boolean } = $props();
  const src = $derived(blobUrl(face.photo, `face:${face.rect.map((v) => v.toFixed(4)).join(',')}` as BlobKind));
</script>

<span class="crop" class:round style="width:{size}px;height:{size}px">
  <img {src} alt="" loading="lazy" />
</span>

<style>
  .crop { position: relative; display: inline-block; overflow: hidden; background: var(--surface-2); flex: none; }
  .crop.round { border-radius: 50%; }
  img { width: 100%; height: 100%; object-fit: cover; display: block; }
</style>
