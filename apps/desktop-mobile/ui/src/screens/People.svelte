<script lang="ts">
  // People (S1-19): named people first, then unnamed groups of faces. Faces are found and
  // grouped on your own devices; nothing leaves them.
  import { onMount } from 'svelte';
  import { api, type PeopleList, type PersonCard } from '../lib/api';
  import { go } from '../lib/route.svelte';
  import { fail, toast } from '../lib/store.svelte';
  import { plural } from '../lib/format';
  import Icon from '../components/Icon.svelte';
  import Button from '../components/Button.svelte';
  import FaceCrop from '../components/FaceCrop.svelte';
  import Sheet from '../components/Sheet.svelte';

  let list = $state<PeopleList | null>(null);
  let naming = $state<PersonCard | null>(null);
  let name = $state('');
  let merging = $state<string[]>([]);
  // Rename, Merge, Hide and This is me live behind ⋯ (or a long press), not under every face.
  let acting = $state<PersonCard | null>(null);
  async function load() {
    try { list = await api.people(); } catch (e) { fail(e); }
  }
  onMount(load);

  async function saveName() {
    if (!naming) return;
    try {
      await api.namePerson(naming.id, name);
      naming = null;
      toast('Named');
      await load();
    } catch (e) { fail(e); }
  }
  function toggleMerge(id: string) {
    merging = merging.includes(id) ? merging.filter((m) => m !== id) : [...merging, id].slice(-2);
  }
  async function merge() {
    const [from, into] = merging;
    try {
      await api.mergePeople(from, into);
      merging = [];
      toast('Merged');
      await load();
    } catch (e) { fail(e); }
  }
</script>

<div class="people">
  {#if list && !list.on}
    <div class="empty">
      <b>People is off</b>
      <span class="muted small">Faces are found and grouped on your own devices and never leave them.</span>
      <Button onclick={async () => { await api.setPeople(true); await load(); }}>Turn on</Button>
    </div>
  {:else if list}
    <p class="small muted intro">Faces are found and grouped on your own devices and never leave them. <button class="link" onclick={async () => { await api.setPeople(false); await load(); }}>Turn off</button></p>
    {#if merging.length === 2}
      <div class="bar"><span class="small">Merge the first into the second?</span><Button size="sm" onclick={merge}>Merge</Button><Button size="sm" kind="ghost" onclick={() => (merging = [])}>Cancel</Button></div>
    {/if}
    {#if list.people.length}
      <div class="section">People</div>
      <div class="grid">
        {#each list.people as p (p.id)}
          <div class="card-p" class:sel={merging.includes(p.id)}>
            <button class="face" aria-label={p.name || 'Unnamed'} onclick={() => go(`person/${p.id}`)} oncontextmenu={(e) => { e.preventDefault(); acting = p; }}><FaceCrop face={p.cover} size={84} /></button>
            <b class="small nm">{p.name || 'Unnamed'}{p.me ? ' (me)' : ''}</b>
            <span class="muted small">{plural(p.count, 'photo')}</span>
            <button class="more" aria-label="Actions for {p.name || 'this person'}" onclick={() => (acting = p)}><Icon name="more" size={18} /></button>
          </div>
        {/each}
      </div>
    {/if}
    {#if list.groups.length}
      <div class="section">Unnamed people</div>
      <div class="grid">
        {#each list.groups as g (g.id)}
          <div class="card-p">
            <button class="face" aria-label="Unnamed person" onclick={() => go(`person/${g.id}`)} oncontextmenu={(e) => { e.preventDefault(); acting = g; }}><FaceCrop face={g.cover} size={84} /></button>
            <span class="muted small">{plural(g.count, 'photo')}</span>
            <button class="more" aria-label="Actions for this person" onclick={() => (acting = g)}><Icon name="more" size={18} /></button>
          </div>
        {/each}
      </div>
    {/if}
    {#if !list.people.length && !list.groups.length}
      <div class="empty"><b>No people yet</b><span class="muted small">Faces show up here once a device has looked through your photos.</span></div>
    {/if}
  {/if}
</div>

<Sheet open={acting !== null} title={acting?.name || 'This person'} onclose={() => (acting = null)} width={340}>
  {#if acting}
    {@const p = acting}
    <div class="menu">
      <button onclick={() => { naming = p; name = p.name; acting = null; }}>{p.name ? 'Rename' : 'Add a name'}</button>
      {#if !p.id.startsWith('group:')}
        <button onclick={() => { toggleMerge(p.id); acting = null; }}>{merging.includes(p.id) ? 'Unselect for merge' : 'Merge with another person…'}</button>
        <button onclick={async () => { await api.setMe(p.id); acting = null; await load(); }}>This is me</button>
        <button onclick={async () => { await api.hidePerson(p.id, true); acting = null; await load(); }}>Hide</button>
      {/if}
    </div>
  {/if}
</Sheet>

<Sheet open={naming !== null} title={naming?.name ? 'Rename' : 'Who is this?'} onclose={() => (naming = null)} width={380}>
  {#if naming}
    <div class="stack">
      <FaceCrop face={naming.cover} size={120} />
      <input class="input" placeholder="Name" bind:value={name} onkeydown={(e) => e.key === 'Enter' && saveName()} />
      <div class="row-flex">
        <Button disabled={!name.trim()} onclick={saveName}>Save</Button>
        {#if !naming.id.startsWith('group:')}
          <Button kind="ghost" onclick={async () => { await api.setMe(naming!.id); naming = null; await load(); }}>This is me</Button>
          <Button kind="ghost" onclick={async () => { await api.hidePerson(naming!.id, true); naming = null; await load(); }}>Hide</Button>
        {/if}
      </div>
    </div>
  {/if}
</Sheet>

<style>
  .people { padding: 16px; overflow: auto; height: 100%; }
  .intro { margin: 0 0 8px; }
  .section { font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 16px 0 8px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(104px, 1fr)); gap: 12px; }
  @media (max-width: 720px) { .grid { grid-template-columns: repeat(3, 1fr); gap: 8px; } }
  .nm { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .more { border: 0; background: none; color: var(--ink-2); cursor: pointer; width: var(--tap); height: 32px; display: grid; place-items: center; border-radius: 8px; }
  .more:hover { background: var(--surface-2); }
  .menu { display: grid; }
  .menu button { text-align: left; border: 0; border-top: 1px solid var(--line); background: none; color: var(--ink); font: inherit; padding: 14px 4px; min-height: var(--tap); cursor: pointer; }
  .menu button:first-child { border-top: 0; }
  .card-p { display: flex; flex-direction: column; align-items: center; gap: 4px; text-align: center; padding: 6px; border-radius: 12px; }
  .card-p.sel { outline: 2px solid var(--accent); }
  .face { border: 0; padding: 0; background: none; cursor: pointer; border-radius: 50%; }
  .link { border: 0; background: none; color: var(--accent); cursor: pointer; padding: 0; }
  .bar { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; }
  .empty { display: flex; flex-direction: column; gap: 8px; align-items: center; justify-content: center; padding: 48px 16px; text-align: center; }
  .stack { display: flex; flex-direction: column; gap: 12px; align-items: center; }
  .row-flex { display: flex; gap: 8px; flex-wrap: wrap; }
</style>
