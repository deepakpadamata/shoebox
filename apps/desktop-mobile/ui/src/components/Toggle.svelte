<script lang="ts">
  let { checked = $bindable(false), label, onchange }: { checked?: boolean; label: string; onchange?: (v: boolean) => void } = $props();
</script>

<button class="toggle" class:on={checked} role="switch" aria-checked={checked} aria-label={label}
  onclick={() => { checked = !checked; onchange?.(checked); }}></button>

<style>
  .toggle { width: 44px; height: 26px; border-radius: 13px; background: var(--surface-3); border: 0; position: relative; flex: none; cursor: pointer; padding: 0; }
  .toggle::after { content: ''; position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; border-radius: 50%; background: #fff; box-shadow: 0 1px 2px rgba(0,0,0,.25); transition: left .12s; }
  .toggle.on { background: var(--ok); }
  .toggle.on::after { left: 21px; }
</style>
