<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props { title?: string; open: boolean; onclose: () => void; width?: number; children: Snippet; }
  import { viewport } from '../lib/viewport.svelte';
  let { title, open, onclose, width = 460, children }: Props = $props();
  function key(e: KeyboardEvent) { if (e.key === 'Escape') onclose(); }
</script>

<svelte:window onkeydown={key} />

{#if open}
  <div class="scrim" role="presentation" onclick={onclose}></div>
  <div class="sheet" class:bottom={viewport.mobile} role="dialog" aria-modal="true" aria-label={title} style={viewport.mobile ? '' : `width:min(${width}px, 94vw)`}>
    {#if title}<div class="head"><b>{title}</b><button class="x" aria-label="Close" onclick={onclose}>✕</button></div>{/if}
    <div class="body">{@render children()}</div>
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; background: var(--scrim); z-index: 40; }
  .sheet { position: fixed; z-index: 41; left: 50%; top: 50%; transform: translate(-50%, -50%); max-height: 90vh; overflow: auto;
    background: var(--surface); border-radius: 18px; box-shadow: var(--shadow); padding: 16px 20px 20px; }
  .sheet.bottom { left: 0; right: 0; top: auto; bottom: 0; transform: none; width: auto; border-radius: 22px 22px 0 0; max-height: 88vh; padding-bottom: calc(24px + var(--safe-bottom)); }
  .sheet.bottom::before { content: ''; display: block; width: 40px; height: 5px; border-radius: 3px; background: var(--surface-3); margin: 0 auto 10px; }
  .head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; font-size: 16px; }
  .x { background: none; border: 0; cursor: pointer; color: var(--ink-3); font-size: 16px; }
</style>
