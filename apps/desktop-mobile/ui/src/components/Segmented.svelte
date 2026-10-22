<script lang="ts">
  interface Props { options: { value: string; label: string }[]; value: string; onchange?: (v: string) => void; }
  let { options, value = $bindable(), onchange }: Props = $props();
</script>

<div class="seg" role="tablist">
  {#each options as o}
    <button role="tab" aria-selected={value === o.value} class:on={value === o.value}
      onclick={() => { value = o.value; onchange?.(o.value); }}>{o.label}</button>
  {/each}
</div>

<style>
  .seg { display: flex; background: var(--surface-2); border-radius: 10px; padding: 3px; gap: 2px; }
  button { flex: 1; text-align: center; padding: 6px 12px; border-radius: 8px; font-size: 12.5px; font-weight: 600; color: var(--ink-2); border: 0; background: transparent; cursor: pointer; }
  button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0,0,0,.12); }
</style>
