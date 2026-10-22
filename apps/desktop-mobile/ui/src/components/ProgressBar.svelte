<script lang="ts">
  interface Segment { value: number; tone: 'ok' | 'warn' | 'risk' | 'info' | 'accent' | 'mute' }
  let { segments, total, label }: { segments: Segment[]; total: number; label?: string } = $props();
</script>

<div class="bar" role="img" aria-label={label}>
  {#each segments as s}
    <i class={s.tone} style="width:{total > 0 ? (s.value / total) * 100 : 0}%"></i>
  {/each}
</div>

<style>
  .bar { height: 8px; background: var(--surface-2); border-radius: 4px; overflow: hidden; display: flex; }
  i { display: block; height: 100%; }
  .ok { background: var(--ok); } .warn { background: var(--warn); } .risk { background: var(--risk); }
  .info { background: var(--info); } .accent { background: var(--accent); } .mute { background: var(--ink-3); }
</style>
