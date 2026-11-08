<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';
  interface Props { tone?: 'risk' | 'warn' | 'info' | 'accent'; icon?: IconName; action?: string; onaction?: () => void; ondismiss?: () => void; children: Snippet; }
  let { tone = 'info', icon = 'warning', action, onaction, ondismiss, children }: Props = $props();
</script>

<div class="banner {tone}" role="status">
  <span class="icon"><Icon name={icon} size={20} /></span>
  <div class="grow small">{@render children()}</div>
  {#if action}<button class="act" onclick={onaction}>{action}</button>{/if}
  {#if ondismiss}<button class="x" aria-label="Dismiss" onclick={ondismiss}><Icon name="close" size={18} /></button>{/if}
</div>

<style>
  .banner { display: flex; gap: 10px; align-items: center; padding: 10px 12px; border-radius: var(--radius); border: 1px solid transparent; }
  .icon { display: flex; }
  .risk { background: var(--risk-soft); border-color: color-mix(in srgb, var(--risk) 25%, transparent); }
  .warn { background: var(--warn-soft); border-color: color-mix(in srgb, var(--warn) 25%, transparent); }
  .info { background: var(--info-soft); border-color: color-mix(in srgb, var(--info) 25%, transparent); }
  .accent { background: var(--accent-soft); border-color: color-mix(in srgb, var(--accent) 25%, transparent); }
  .act { background: none; border: 0; font-weight: 600; cursor: pointer; color: var(--accent); }
  .risk .act { color: var(--risk); }
  .x { background: none; border: 0; cursor: pointer; color: var(--ink-3); }
</style>
