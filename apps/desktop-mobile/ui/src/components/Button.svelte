<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props {
    kind?: 'default' | 'primary' | 'ghost' | 'danger';
    size?: 'md' | 'sm';
    block?: boolean;
    disabled?: boolean;
    title?: string;
    type?: 'button' | 'submit';
    onclick?: (e: MouseEvent) => void;
    children: Snippet;
  }
  let { kind = 'default', size = 'md', block = false, disabled = false, title, type = 'button', onclick, children }: Props = $props();
</script>

<button class="btn {kind} {size}" class:block {disabled} {title} {type} {onclick}>{@render children()}</button>

<style>
  .btn {
    display: inline-flex; align-items: center; justify-content: center; gap: 6px;
    height: 40px; padding: 0 16px; border-radius: 12px; border: 1px solid var(--line);
    background: var(--surface); font-weight: 600; font-size: 14px; cursor: pointer;
  }
  .btn:hover:not(:disabled) { background: var(--surface-2); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn.sm { height: 30px; padding: 0 11px; font-size: 13px; border-radius: 9px; }
  .btn.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.06); background: var(--accent); }
  .btn.ghost { background: transparent; border-color: transparent; color: var(--accent); }
  .btn.danger { color: var(--risk); }
  .block { display: flex; width: 100%; }
</style>
