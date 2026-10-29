<script lang="ts">
  // The app icon (design/icon/shoebox-icon.svg), drawn inline so it needs no asset.
  let { size = 24 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
  <rect width="100" height="100" rx="22" fill="#2b5f5a" />
  <g transform="rotate(-10 50 50)">
    <circle cx="27" cy="35" r="4.5" fill="#f2a33a" />
    <rect x="14" y="24" width="46" height="32" rx="3" fill="none" stroke="#fbeedf" stroke-width="6" />
    <rect x="15" y="50" width="44" height="6" fill="#fbeedf" />
    <rect x="40" y="44" width="46" height="32" rx="3" fill="none" stroke="#2b5f5a" stroke-width="10" />
    <path d="M43 70 L53 61 L60 66 L69 54 L83 68 L83 70 Z" fill="#8cc0b9" />
    <rect x="40" y="44" width="46" height="32" rx="3" fill="none" stroke="#fbeedf" stroke-width="6" />
    <rect x="41" y="70" width="44" height="6" fill="#fbeedf" />
    <line x1="60" y1="38" x2="60" y2="49" stroke="#2b5f5a" stroke-width="10" />
    <line x1="60" y1="36" x2="60" y2="50" stroke="#fbeedf" stroke-width="6" />
  </g>
</svg>

<style>
  svg { display: block; flex: none; }
</style>
