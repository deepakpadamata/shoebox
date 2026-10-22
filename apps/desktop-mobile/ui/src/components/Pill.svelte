<script lang="ts">
  import type { Snippet } from 'svelte';
  let { tone = 'mute', children }: { tone?: 'ok' | 'warn' | 'risk' | 'info' | 'mute'; children: Snippet } = $props();
</script>

<span class="pill {tone}">{@render children()}</span>

<style>
  .pill { display: inline-flex; align-items: center; gap: 5px; font-size: 11.5px; font-weight: 600; padding: 3px 8px; border-radius: 999px; white-space: nowrap; }
  .ok { background: var(--ok-soft); color: var(--ok); }
  .warn { background: var(--warn-soft); color: var(--warn); }
  .risk { background: var(--risk-soft); color: var(--risk); }
  .info { background: var(--info-soft); color: var(--info); }
  .mute { background: var(--surface-2); color: var(--ink-2); }
</style>
