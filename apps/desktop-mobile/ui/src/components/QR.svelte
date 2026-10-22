<script lang="ts">
  // The app core renders the QR code as an SVG document (it carries a pairing ticket).
  let { svg, size = 190 }: { svg: string; size?: number } = $props();
</script>

<div class="qr" style="width:{size}px;height:{size}px" role="img" aria-label="QR code">{@html svg}</div>

<style>
  .qr { padding: 10px; background: #fff; border-radius: 16px; border: 1px solid var(--line); }
  .qr :global(svg) { width: 100%; height: 100%; display: block; }
</style>
