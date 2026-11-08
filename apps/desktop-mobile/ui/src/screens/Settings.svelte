<script lang="ts">
  // Settings (U1-14, U1-15). On a phone: a home that lists the groups with their current value,
  // Backup first, and a page per group (`#/settings/<group>`, so Back is predictable). On a
  // computer: two panes, the groups on the left with a filter, the group on the right.
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import { go, rest } from '../lib/route.svelte';
  import { app } from '../lib/store.svelte';
  import { safetyLine } from '../lib/safety';
  import { isHandheld } from '../lib/pick';
  import { savedTheme } from '../lib/theme';
  import { viewport } from '../lib/viewport.svelte';
  import Icon, { type IconName } from '../components/Icon.svelte';
  import { filterGroups, GROUPS, groupTitle } from './settings/groups';
  import { cfg, loadSettings } from './settings/store.svelte';
  import About from './settings/About.svelte';
  import Advanced from './settings/Advanced.svelte';
  import Appearance from './settings/Appearance.svelte';
  import Camera from './settings/Camera.svelte';
  import Recovery from './settings/Recovery.svelte';
  import SearchPeople from './settings/SearchPeople.svelte';
  import You from './settings/You.svelte';

  const group = $derived(rest()[0]);
  const known = $derived(GROUPS.some((g) => g.id === group));
  // A computer always shows one group; the first until another is chosen.
  const shown = $derived(known ? group : 'camera');
  const line = $derived(safetyLine(app.backup_status, isHandheld() ? 'phone' : 'computer'));
  let filter = $state('');
  const matches = $derived(filterGroups(filter));
  let version = $state('');
  onMount(() => {
    loadSettings();
    api.about().then((a) => (version = a.version)).catch(() => {});
  });
  const themeName = $derived(({ system: 'System', light: 'Light', dark: 'Dark' } as Record<string, string>)[savedTheme()] ?? 'System');
  const icons: Record<string, IconName> = { camera: 'camera', you: 'person', search: 'search', appearance: 'settings', recovery: 'key', advanced: 'lock', about: 'info' };
  const value = (id: string) => (id === 'you' ? (cfg.s?.device_name ?? '') : id === 'appearance' ? themeName : id === 'about' ? version : '');
  const open = (id: string) => go(`settings/${id}`);
</script>

{#snippet content(id: string)}
  {#if id === 'camera'}<Camera />
  {:else if id === 'you'}<You />
  {:else if id === 'search'}<SearchPeople />
  {:else if id === 'appearance'}<Appearance />
  {:else if id === 'recovery'}<Recovery />
  {:else if id === 'advanced'}<Advanced />
  {:else if id === 'about'}<About />{/if}
{/snippet}

{#if viewport.mobile}
  <div class="page">
    {#if known}
      {@render content(group)}
    {:else}
      <div class="set-section">Backup</div>
      <div class="card list">
        <button class="item" onclick={() => go('backup')}>
          <span class="dot {line.tone}"></span><span class="grow"><b>{line.short}</b><span class="sub">Devices, camera backup and storage</span></span><Icon name="chevron" size={16} />
        </button>
      </div>

      <div class="set-section">App</div>
      <div class="card list">
        {#each GROUPS.filter((g) => ['search', 'appearance', 'you', 'recovery'].includes(g.id)) as g (g.id)}
          <button class="item" onclick={() => open(g.id)}><Icon name={icons[g.id]} size={22} /><span class="grow">{g.title}</span><span class="muted small val">{value(g.id)}</span><Icon name="chevron" size={16} /></button>
        {/each}
      </div>

      <div class="set-section">More</div>
      <div class="card list">
        {#each GROUPS.filter((g) => ['advanced', 'about'].includes(g.id)) as g (g.id)}
          <button class="item" onclick={() => open(g.id)}><Icon name={icons[g.id]} size={22} /><span class="grow">{g.title}</span><span class="muted small val">{value(g.id)}</span><Icon name="chevron" size={16} /></button>
        {/each}
      </div>
    {/if}
  </div>
{:else}
  <div class="two">
    <nav class="left" aria-label="Settings">
      <input class="filter" type="search" placeholder="Filter settings" aria-label="Filter settings" bind:value={filter}
        onkeydown={(e) => { if (e.key === 'Enter' && matches[0]) { open(matches[0].id); } if (e.key === 'Escape') filter = ''; }} />
      <button class="gl status" onclick={() => go('backup')}><span class="dot {line.tone}"></span><span class="grow">{line.short}</span><Icon name="chevron" size={14} /></button>
      {#each matches as g (g.id)}
        <button class="gl" class:on={shown === g.id} onclick={() => open(g.id)}><Icon name={icons[g.id]} size={18} />{g.title}</button>
      {:else}
        <p class="muted small nomatch">No setting matches “{filter}”.</p>
      {/each}
    </nav>
    <div class="right">{#key shown}{@render content(shown)}{/key}</div>
  </div>
{/if}

<style>
  .page { padding: 12px 14px calc(16px + var(--safe-bottom)); overflow: auto; height: 100%; }
  .gap { margin-top: 10px; }
  .list { padding: 0; overflow: hidden; }
  .item { display: flex; align-items: center; gap: 12px; width: 100%; min-height: var(--tap); padding: 6px 14px; border: 0; background: none; font: inherit; font-size: var(--t-md); color: var(--ink); text-align: left; cursor: pointer; }
  .item + .item { border-top: 1px solid var(--line); }
  .item .val { max-width: 40%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grow { flex: 1; min-width: 0; }
  .sub { display: block; font-size: var(--t-sm); color: var(--ink-2); }
  .dot { width: 12px; height: 12px; border-radius: 50%; flex: none; background: var(--ink-3); }
  .dot.ok { background: var(--ok); } .dot.info { background: var(--info); } .dot.warn { background: var(--warn); } .dot.risk { background: var(--risk); }
  .two { display: flex; height: 100%; }
  .left { width: 240px; flex: none; border-right: 1px solid var(--line); padding: 14px 10px; display: flex; flex-direction: column; gap: 2px; overflow: auto; }
  .filter { height: 34px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; margin-bottom: 8px; color: var(--ink); font: inherit; }
  .gl { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border: 0; border-radius: 8px; background: none; font: inherit; font-size: var(--t-sm); color: var(--ink); cursor: pointer; text-align: left; }
  .gl:hover { background: var(--surface-2); }
  .gl.on { background: var(--surface-3); font-weight: 600; }
  .gl.status { margin-bottom: 6px; border: 1px solid var(--line); }
  .nomatch { padding: 8px 10px; }
  .right { flex: 1; min-width: 0; padding: 18px 24px; overflow: auto; max-width: 760px; }
</style>
