<script lang="ts">
  // Search results (S1-05, mockups 8b–8d): the recognised pieces as removable chips, a count and
  // what the search covers, then the timeline grid. On a phone the screen has its own field;
  // on the desktop the field lives in the toolbar.
  import type { SearchChip } from '../lib/api';
  import { go, param } from '../lib/route.svelte';
  import { chipIcon, coverageLine, queryFromParam, removeChip, searchPath } from '../lib/search';
  import { api, type SearchStatus } from '../lib/api';
  import { plural } from '../lib/format';
  import SearchField from '../components/SearchField.svelte';
  import Library from './Library.svelte';

  /** `similar`: show the photos that look like this one (S1-12) instead of a query's results. */
  let { mobile = false, similar }: { mobile?: boolean; similar?: string } = $props();

  const query = $derived(queryFromParam(param()));
  let text = $state('');
  $effect(() => {
    text = query;
  });
  let chips = $state<SearchChip[]>([]);
  let words = $state('');
  let total = $state<number | null>(null);
  let ranked = $state(false);
  // What searching by content covers so far (S1-16), asked again with each search.
  let status = $state<SearchStatus | null>(null);
  $effect(() => {
    query;
    api.searchStatus().then((s) => (status = s)).catch(() => {});
  });
  const coverage = $derived(coverageLine(status));

  function run(q: string) {
    if (q === query) return;
    total = null;
    go(searchPath(q));
  }

  function drop(c: SearchChip) {
    run(removeChip(query, c));
  }

  const summary = $derived(
    total === null || !query.trim() ? '' : `${ranked ? 'Best matches · ' : ''}${plural(total, 'photo')}`,
  );
</script>

<div class="search">
  {#if mobile}
    <div class="bar">
      <div class="grow"><SearchField bind:value={text} onsubmit={run} oncancel={() => go('library')} autofocus={!query} /></div>
      <button class="cancel" onclick={() => go('library')}>Cancel</button>
    </div>
  {/if}
  {#if similar}
    <div class="coverage small muted" role="status">
      {total === null ? 'Looking for photos like this one…' : total === 0 ? 'No similar photos on this device yet' : `Photos like this one · ${plural(total, 'photo')}`}
    </div>
  {:else if query.trim()}
    <div class="chips">
      {#each chips as c (c.start)}
        <span class="chip">
          <span aria-hidden="true">{chipIcon(c.kind)}</span>{c.label}
          <button class="x" aria-label="Remove {c.label}" onclick={() => drop(c)}>✕</button>
        </span>
      {/each}
      {#if words}<span class="words muted small">{chips.length ? 'and ' : ''}the words “{words}”</span>{/if}
    </div>
    <div class="coverage small muted" role="status">
      {summary}{summary && coverage ? ' · ' : ''}{coverage}
    </div>
  {/if}
  <div class="grid">
    {#if similar}
      {#key similar}
        <Library source="similar" photo={similar} onsearch={(p) => { total = p.total; ranked = true; }} />
      {/key}
    {:else if query.trim()}
      {#key query}
        <Library source="search" {query} onsearch={(p) => { chips = p.filters; words = p.free_text; total = p.total; ranked = p.ranked; }} />
      {/key}
    {:else}
      <div class="tips">
        <b>Search your library</b>
        <ul class="muted small">
          <li>A year, a month or a season: "2019", "March", "last summer"</li>
          <li>A place, a camera or an album: "Lisbon", "Pixel 8"</li>
          <li>"videos", "screenshots", "selfies", "favourites"</li>
        </ul>
        <span class="muted small">Searches run on this device, even with no other device online.</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .search { height: 100%; display: flex; flex-direction: column; min-height: 0; }
  .bar { display: flex; gap: 10px; align-items: center; padding: 6px 16px 8px; }
  .cancel { border: 0; background: transparent; color: var(--accent); font: inherit; font-size: 15px; cursor: pointer; padding: 0; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; padding: 10px 18px 4px; align-items: center; }
  .chip { display: inline-flex; align-items: center; gap: 6px; padding: 3px 4px 3px 10px; border-radius: 999px; background: var(--accent-soft); font-size: 12.5px; font-weight: 600; }
  .x { border: 0; background: transparent; color: var(--ink-3); cursor: pointer; font-size: 11px; width: 22px; height: 22px; border-radius: 50%; }
  .x:hover { background: var(--surface-3); color: var(--ink); }
  .coverage { padding: 2px 18px 8px; }
  .grid { flex: 1; min-height: 0; }
  .tips { height: 100%; display: grid; place-content: center; text-align: center; gap: 8px; padding: 20px; color: var(--ink-2); }
  .tips ul { text-align: left; margin: 0 auto; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
</style>
