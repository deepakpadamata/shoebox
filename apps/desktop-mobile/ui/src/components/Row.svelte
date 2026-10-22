<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props { icon?: string; title: string; subtitle?: string; onclick?: () => void; lead?: Snippet; trail?: Snippet; }
  let { icon, title, subtitle, onclick, lead, trail }: Props = $props();
</script>

<svelte:element this={onclick ? 'button' : 'div'} class="row" class:click={!!onclick} role={onclick ? undefined : 'listitem'} {onclick}>
  {#if lead}{@render lead()}{:else if icon}<span class="ico" aria-hidden="true">{icon}</span>{/if}
  <span class="grow"><span class="t1">{title}</span>{#if subtitle}<span class="t2">{subtitle}</span>{/if}</span>
  {#if trail}{@render trail()}{/if}
</svelte:element>

<style>
  .row { display: flex; align-items: center; gap: 12px; padding: 11px 14px; border: 0; border-top: 1px solid var(--line); background: transparent; width: 100%; text-align: left; }
  .row:first-child { border-top: 0; }
  .click { cursor: pointer; }
  .click:hover { background: var(--surface-2); }
  .ico { width: 36px; height: 36px; border-radius: 10px; flex: none; display: grid; place-items: center; background: var(--surface-2); font-size: 18px; }
  .t1 { display: block; font-weight: 600; font-size: 14.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .t2 { display: block; font-size: 12.5px; color: var(--ink-2); }
</style>
