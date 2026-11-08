<script lang="ts">
  // The search field (S1-05, mockups 8a/8d): suggestions from the library's own names while
  // typing, recent searches (this device only) when empty, arrows to move, Enter to search,
  // Esc to clear and then to close.
  import { onDestroy } from 'svelte';
  import Icon from './Icon.svelte';
  import { api, type SearchSuggestion } from '../lib/api';
  import { addRecent, clearRecent, complete, debounce, groupSuggestions, lastWord, loadRecent, step } from '../lib/search';

  interface Props {
    value: string;
    /** Run the search (`q` may be empty). */
    onsubmit: (q: string) => void;
    /** Esc on an empty field, or Cancel. */
    oncancel?: () => void;
    autofocus?: boolean;
    /** `/` and ⌘F / Ctrl+F focus this field from anywhere (desktop). */
    shortcut?: boolean;
    placeholder?: string;
  }
  let { value = $bindable(), onsubmit, oncancel, autofocus = false, shortcut = false, placeholder = 'Search photos' }: Props = $props();

  let input: HTMLInputElement | undefined = $state();
  let focused = $state(false);
  let open = $state(false);
  let suggestions = $state<SearchSuggestion[]>([]);
  let recent = $state<string[]>(loadRecent());
  let at = $state(-1);

  // One flat list for the arrows: recent searches when the field is empty, else suggestions.
  type Entry = { kind: 'recent'; text: string } | { kind: 'name'; s: SearchSuggestion };
  const groups = $derived(groupSuggestions(suggestions));
  const entries = $derived<Entry[]>(
    value.trim() ? groups.flatMap((g) => g.items.map((s) => ({ kind: 'name' as const, s }))) : recent.map((text) => ({ kind: 'recent' as const, text })),
  );
  const showList = $derived(open && focused && entries.length > 0);

  let asked = 0;
  const fetchSuggestions = debounce(async (prefix: string) => {
    const mine = ++asked;
    if (!prefix) {
      suggestions = [];
      return;
    }
    const got = await api.searchSuggestions(prefix, 12).catch(() => []);
    if (mine === asked) suggestions = got;
  }, 120);
  onDestroy(() => fetchSuggestions.cancel());

  function oninput() {
    open = true;
    at = -1;
    fetchSuggestions(lastWord(value));
  }

  function submit(q: string) {
    open = false;
    at = -1;
    if (q.trim()) recent = addRecent(q);
    // The keyboard would otherwise stay over the results.
    input?.blur();
    onsubmit(q.trim());
  }

  function choose(e: Entry) {
    if (e.kind === 'recent') value = e.text;
    else value = complete(value, e.s.name);
    submit(value);
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      open = true;
      at = step(at, e.key === 'ArrowDown' ? 1 : -1, entries.length);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (showList && at >= 0 && entries[at]) choose(entries[at]);
      else submit(value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (showList) {
        open = false;
        at = -1;
      } else if (value) {
        value = '';
        suggestions = [];
        onsubmit('');
      } else {
        input?.blur();
        oncancel?.();
      }
    }
  }

  function globalKey(e: KeyboardEvent) {
    if (!shortcut) return;
    const t = e.target as HTMLElement | null;
    const typing = t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t?.isContentEditable;
    const find = (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'f';
    if (find || (e.key === '/' && !typing)) {
      e.preventDefault();
      input?.focus();
      input?.select();
    }
  }

  $effect(() => {
    if (autofocus) input?.focus();
  });

  export function focus() {
    input?.focus();
  }
</script>

<svelte:window onkeydown={globalKey} />

<div class="field" class:focused>
  <span class="glass" aria-hidden="true"><Icon name="search" size={18} /></span>
  <input bind:this={input} bind:value type="search" {placeholder} aria-label="Search photos" autocomplete="off" spellcheck="false"
    role="combobox" aria-expanded={showList} aria-controls="search-suggestions" aria-activedescendant={at >= 0 ? `sugg-${at}` : undefined}
    {oninput} onkeydown={keydown}
    onfocus={() => { focused = true; open = true; recent = loadRecent(); }}
    onblur={() => setTimeout(() => (focused = false), 150)} />
  {#if value}
    <button class="clear" aria-label="Clear" onmousedown={(e) => e.preventDefault()} onclick={() => { value = ''; suggestions = []; input?.focus(); onsubmit(''); }}><Icon name="close" size={16} /></button>
  {:else if shortcut}
    <kbd class="hint" aria-hidden="true">/</kbd>
  {/if}

  {#if showList}
    <div class="list" id="search-suggestions" role="listbox">
      {#if !value.trim()}
        <div class="g">Recent <button class="link" onmousedown={(e) => e.preventDefault()} onclick={() => { clearRecent(); recent = []; }}>Clear</button></div>
        {#each recent as text, i}
          <button id="sugg-{i}" role="option" aria-selected={at === i} class="r" class:on={at === i} onmousedown={(e) => e.preventDefault()} onclick={() => choose({ kind: 'recent', text })}>
            <span class="i" aria-hidden="true"><Icon name="clock" size={16} /></span>{text}
          </button>
        {/each}
      {:else}
        {@const offsets = groups.map((_, gi) => groups.slice(0, gi).reduce((n, g) => n + g.items.length, 0))}
        {#each groups as g, gi}
          <div class="g">{g.title}</div>
          {#each g.items as s, k}
            {@const i = offsets[gi] + k}
            <button id="sugg-{i}" role="option" aria-selected={at === i} class="r" class:on={at === i} onmousedown={(e) => e.preventDefault()} onclick={() => choose({ kind: 'name', s })}>
              <span class="i" aria-hidden="true"><Icon name={g.icon} size={16} /></span>{s.name}
            </button>
          {/each}
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .field { position: relative; display: flex; align-items: center; gap: 6px; height: 34px; padding: 0 10px; border-radius: 10px;
    background: var(--surface-2); border: 1px solid var(--line); min-width: 0; }
  .field.focused { border-color: var(--accent); }
  .glass { color: var(--ink-3); font-size: 15px; }
  input { flex: 1; min-width: 0; border: 0; background: transparent; color: var(--ink); font: inherit; font-size: 14px; outline: none; height: 100%; }
  input::-webkit-search-cancel-button { display: none; }
  .clear { border: 0; background: transparent; color: var(--ink-3); cursor: pointer; padding: 4px; font-size: 13px; }
  .hint { font: inherit; font-size: var(--t-xs); color: var(--ink-3); border: 1px solid var(--line); border-radius: 5px; padding: 0 5px; }
  .list { position: absolute; z-index: 20; top: calc(100% + 6px); left: 0; right: 0; max-height: 60vh; overflow: auto; padding: 6px 0;
    background: var(--surface); border: 1px solid var(--line); border-radius: 12px; box-shadow: var(--shadow); }
  .g { display: flex; justify-content: space-between; font-size: var(--t-xs); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.05em; padding: 8px 12px 4px; }
  .link { border: 0; background: transparent; color: var(--accent); font: inherit; font-size: var(--t-xs); cursor: pointer; text-transform: none; letter-spacing: 0; }
  .r { display: flex; width: 100%; gap: 8px; align-items: center; padding: 7px 12px; border: 0; background: transparent; color: var(--ink); font: inherit; font-size: 13.5px; text-align: left; cursor: pointer; }
  .r.on, .r:hover { background: var(--surface-3); }
  .i { width: 18px; text-align: center; }
</style>
