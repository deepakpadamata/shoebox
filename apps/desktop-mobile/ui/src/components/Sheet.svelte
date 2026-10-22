<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props { title?: string; open: boolean; onclose: () => void; width?: number; children: Snippet; }
  import { viewport } from '../lib/viewport.svelte';
  let { title, open, onclose, width = 460, children }: Props = $props();
  let box: HTMLDivElement | undefined = $state();
  let opener: Element | null = null;
  const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
  // A dialog takes focus when it opens, keeps it inside (Tab wraps) and gives it back on close.
  $effect(() => {
    if (open && box) {
      opener = document.activeElement;
      const first = box.querySelector<HTMLElement>('input, textarea, ' + FOCUSABLE) ?? box;
      first.focus({ preventScroll: true });
      return () => { (opener as HTMLElement | null)?.focus?.({ preventScroll: true }); };
    }
  });
  function key(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === 'Escape') onclose();
    if (e.key === 'Tab' && box) {
      const items = [...box.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (items.length === 0) return;
      const [first, last] = [items[0], items[items.length - 1]];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  }
</script>

<svelte:window onkeydown={key} />

{#if open}
  <div class="scrim" role="presentation" onclick={onclose}></div>
  <div class="sheet" bind:this={box} tabindex="-1" class:bottom={viewport.mobile} role="dialog" aria-modal="true" aria-label={title} style={viewport.mobile ? '' : `width:min(${width}px, 94vw)`}>
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
