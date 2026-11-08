<script lang="ts">
  // Search results (S1-05, mockups 8b–8d): the recognised pieces as removable chips, a count and
  // what the search covers, then the timeline grid. On a phone the screen has its own field;
  // on the desktop the field lives in the toolbar.
  import type { SearchChip } from '../lib/api';
  import { go, param } from '../lib/route.svelte';
  import { loadRecent, chipIcon, coverageLine, queryFromParam, relaxSuggestion, removeChip, searchPath } from '../lib/search';
  import { api, type PersonCard, type SearchStatus } from '../lib/api';
  import { plural } from '../lib/format';
  import Icon from '../components/Icon.svelte';
  import { onMount } from 'svelte';
  import FaceCrop from '../components/FaceCrop.svelte';
  import SearchField from '../components/SearchField.svelte';
  import Library from './Library.svelte';

  /** `similar`: show the photos that look like this one (S1-12) instead of a query's results. */
  let { mobile = false, similar }: { mobile?: boolean; similar?: string } = $props();

  // The empty search: what is worth searching for (U1-09).
  let recent = $state<string[]>(loadRecent());
  let folks = $state<PersonCard[]>([]);
  let places = $state<string[]>([]);
  onMount(() => {
    api.people().then((l) => (folks = l.people.filter((p) => p.name && !p.hidden).slice(0, 12))).catch(() => {});
    api.searchSuggestions('', 40).then((l) => (places = l.filter((x) => x.kind === 'place').map((x) => x.name).slice(0, 10))).catch(() => {});
  });

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

  const relax = $derived(total === 0 && !similar ? relaxSuggestion(chips) : null);

  const summary = $derived(
    total === null || !query.trim() ? '' : `${ranked ? 'Best matches · ' : ''}${plural(total, 'photo')}`,
  );
</script>

<div class="search">
  {#if mobile}
    <div class="bar">
      <div class="grow"><SearchField bind:value={text} onsubmit={run} oncancel={() => go('library')} autofocus={!query} /></div>
    </div>
  {/if}
  {#if similar}
    <div class="coverage small muted" role="status">
      {total === null ? 'Looking for photos like this one…' : total === 0 ? 'No similar photos on this device yet' : `Photos like this one · ${plural(total, 'photo')}`}
    </div>
  {:else if query.trim()}
    <div class="chips">
      {#each chips as c (c.start)}
        {@const ic = chipIcon(c.kind)}
        <span class="chip">
          {#if ic}<Icon name={ic} size={14} />{/if}{c.label}
          <button class="x" aria-label="Remove {c.label}" onclick={() => drop(c)}><Icon name="close" size={12} /></button>
        </span>
      {/each}
      {#if words}<span class="words muted small">{chips.length ? 'and ' : ''}the words “{words}”</span>{/if}
    </div>
    <div class="coverage small muted" role="status">
      {summary}{summary && coverage ? ' · ' : ''}{coverage}
    </div>
    {#if relax}
      <div class="relax"><button class="link" onclick={() => drop(relax.chip)}>{relax.text}</button></div>
    {/if}
  {/if}
  <div class="grid" onscrollcapture={() => (document.activeElement as HTMLElement | null)?.blur?.()}>
    {#if similar}
      {#key similar}
        <Library source="similar" photo={similar} onsearch={(p) => { total = p.total; ranked = true; }} />
      {/key}
    {:else if query.trim()}
      {#key query}
        <Library source="search" {query} onsearch={(p) => { chips = p.filters; words = p.free_text; total = p.total; ranked = p.ranked; }} />
      {/key}
    {:else}
      <div class="home">
        {#if recent.length}
          <div class="hsec">Recent searches</div>
          <div class="chiprow">{#each recent.slice(0, 8) as r (r)}<button class="hchip" onclick={() => run(r)}><Icon name="clock" size={14} />{r}</button>{/each}</div>
        {/if}
        {#if folks.length}
          <div class="hsec">People</div>
          <div class="faces">{#each folks as p (p.id)}<button class="face" onclick={() => run(p.name)}><FaceCrop face={p.cover} size={56} /><span class="small">{p.name}</span></button>{/each}</div>
        {/if}
        {#if places.length}
          <div class="hsec">Places</div>
          <div class="chiprow">{#each places as pl (pl)}<button class="hchip" onclick={() => run(pl)}><Icon name="place" size={14} />{pl}</button>{/each}</div>
        {/if}
        <div class="hsec">Try</div>
        <ul class="muted small">
          <li>A year, a month or a season: "2019", "March", "last summer"</li>
          <li>A place, a camera or an album: "Lisbon", "Pixel 8"</li>
          <li>"videos", "screenshots", "selfies", "favorites"</li>
        </ul>
        <span class="muted small">Searches run on this device, even with no other device online.</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .search { height: 100%; display: flex; flex-direction: column; min-height: 0; }
  .bar { display: flex; gap: 10px; align-items: center; padding: 6px 16px 8px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; padding: 10px 18px 4px; align-items: center; }
  .chip { display: inline-flex; align-items: center; gap: 6px; padding: 3px 4px 3px 10px; border-radius: 999px; background: var(--accent-soft); font-size: var(--t-sm); font-weight: 600; }
  .x { border: 0; background: transparent; color: var(--ink-3); cursor: pointer; font-size: var(--t-xs); width: 22px; height: 22px; border-radius: 50%; }
  .x:hover { background: var(--surface-3); color: var(--ink); }
  .coverage { padding: 2px 18px 8px; }
  .relax { padding: 0 18px 8px; }
  .link { border: 0; background: transparent; color: var(--accent); font: inherit; font-size: 14px; font-weight: 600; cursor: pointer; padding: 4px 0; }
  .grid { flex: 1; min-height: 0; }
  .home { height: 100%; overflow: auto; padding: 8px 18px 24px; display: grid; align-content: start; gap: 8px; }
  .hsec { font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin-top: 10px; }
  .chiprow { display: flex; flex-wrap: wrap; gap: 8px; }
  .hchip { display: inline-flex; align-items: center; gap: 6px; min-height: 36px; padding: 0 12px; border: 1px solid var(--line); border-radius: 99px; background: var(--surface); color: var(--ink); font: inherit; font-size: var(--t-sm); cursor: pointer; }
  .faces { display: flex; gap: 14px; overflow-x: auto; padding-bottom: 4px; }
  .face { display: grid; justify-items: center; gap: 4px; border: 0; background: none; color: var(--ink); cursor: pointer; min-width: 64px; }
</style>
