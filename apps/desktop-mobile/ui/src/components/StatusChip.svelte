<script lang="ts">
  // The backup answer as a small control (U1-08): a chip in the Photos header on the phone, a row
  // in the sidebar footer on the desktop. Both read `app.backup_status`, so they always say the
  // same sentence in the same colour.
  import { app } from '../lib/store.svelte';
  import { safetyLine } from '../lib/safety';
  import { isHandheld } from '../lib/pick';
  import Icon, { type IconName } from './Icon.svelte';

  let { variant = 'chip', onclick }: { variant?: 'chip' | 'row'; onclick?: () => void } = $props();
  const line = $derived(safetyLine(app.backup_status, isHandheld() ? 'phone' : 'computer'));
  const icon = $derived<IconName>(({ ok: 'check', neutral: 'info', info: 'sync', warn: 'pause', risk: 'warning' } as const)[line.tone]);
</script>

<button class="s {variant} {line.tone}" {onclick} aria-label="Backup: {line.short}">
  <Icon name={icon} size={variant === 'chip' ? 16 : 18} />
  <span class="txt">{line.short}</span>
  {#if variant === 'row'}<Icon name="chevron" size={14} />{/if}
</button>

<style>
  .s { display: inline-flex; align-items: center; gap: 6px; border: 0; font: inherit; font-weight: 600; cursor: pointer; white-space: nowrap; color: var(--c); background: var(--bg-c); }
  .chip { padding: 6px 12px 6px 9px; border-radius: 99px; font-size: var(--t-sm); max-width: 100%; }
  .chip .txt { overflow: hidden; text-overflow: ellipsis; }
  .row { width: 100%; padding: 10px 12px; border-radius: 10px; font-size: var(--t-sm); justify-content: flex-start; text-align: left; }
  .row .txt { flex: 1; overflow: hidden; text-overflow: ellipsis; }
  .ok { --c: var(--ok); --bg-c: var(--ok-soft); }
  .neutral { --c: var(--ink-2); --bg-c: var(--surface-2); }
  .info { --c: var(--info); --bg-c: var(--info-soft); }
  .warn { --c: var(--warn); --bg-c: var(--warn-soft); }
  .risk { --c: var(--risk); --bg-c: var(--risk-soft); }
</style>
