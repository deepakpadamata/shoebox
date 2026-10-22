<script lang="ts">
  import type { Snippet } from 'svelte';
  let { children }: { children: Snippet } = $props();
</script>

<div class="list">{@render children()}</div>

<style>
  .list { background: var(--surface); border-radius: var(--radius); border: 1px solid var(--line); overflow: hidden; }
</style>
