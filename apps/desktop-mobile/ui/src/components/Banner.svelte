<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props { tone?: 'risk' | 'warn' | 'info' | 'accent'; icon?: string; action?: string; onaction?: () => void; ondismiss?: () => void; children: Snippet; }
  let { tone = 'info', icon = '⚠︎', action, onaction, ondismiss, children }: Props = $props();
</script>

<div class="banner {tone}" role="status">
  <span class="icon" aria-hidden="true">{icon}</span>
  <div class="grow small">{@render children()}</div>
  {#if action}<button class="act" onclick={onaction}>{action}</button>{/if}
  {#if ondismiss}<button class="x" aria-label="Dismiss" onclick={ondismiss}>✕</button>{/if}
</div>

<style>
  .banner { display: flex; gap: 10px; align-items: center; padding: 10px 12px; border-radius: var(--radius); border: 1px solid transparent; }
  .icon { font-size: 18px; }
  .risk { background: var(--risk-soft); border-color: color-mix(in srgb, var(--risk) 25%, transparent); }
  .warn { background: var(--warn-soft); border-color: color-mix(in srgb, var(--warn) 25%, transparent); }
  .info { background: var(--info-soft); border-color: color-mix(in srgb, var(--info) 25%, transparent); }
  .accent { background: var(--accent-soft); border-color: color-mix(in srgb, var(--accent) 25%, transparent); }
  .act { background: none; border: 0; font-weight: 600; cursor: pointer; color: var(--accent); }
  .risk .act { color: var(--risk); }
  .x { background: none; border: 0; cursor: pointer; color: var(--ink-3); }
</style>
