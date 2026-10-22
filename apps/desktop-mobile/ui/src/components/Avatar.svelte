<script lang="ts">
  let { name, size = 32, hue }: { name: string; size?: number; hue?: number } = $props();
  const initial = $derived((name.trim()[0] ?? '?').toUpperCase());
  // A stable colour per name keeps the same person recognisable.
  const h = $derived(hue ?? [...name].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 20));
</script>

<span class="avatar" style="width:{size}px;height:{size}px;font-size:{size * 0.4}px;background:hsl({h} 45% 42%)" aria-hidden="true">{initial}</span>

<style>
  .avatar { border-radius: 50%; flex: none; display: inline-grid; place-items: center; font-weight: 700; color: #fff; }
</style>
