<script lang="ts">
  // Library health and free up space (mockup 6a).
  import { onMount, untrack } from 'svelte';
  import { api, type Health } from '../lib/api';
  import { bytes, plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import Banner from '../components/Banner.svelte';
  import Button from '../components/Button.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import Sheet from '../components/Sheet.svelte';

  let health = $state<Health | null>(null);
  let confirm = $state(false);
  let busy = $state(false);

  async function load() { health = await api.health(); }
  onMount(() => { load().catch(fail); });
  $effect(() => {
    app.version;
    untrack(() => { load().catch(() => {}); });
  });

  const bucketLabel = (i: number, n: number) => (i === n - 1 ? `${i}+ copies` : i === 0 ? 'No copy' : i === 1 ? '1 copy' : `${i} copies`);
  const bucketTone = (i: number, target: number) => (i === 0 ? 'risk' : i < target ? 'warn' : 'ok') as 'risk' | 'warn' | 'ok';

  async function free() {
    busy = true;
    try {
      const r = await api.freeUpSpace(false);
      toast(`Freed ${bytes(r.bytes)} from ${plural(r.photos, 'photo')}`);
      confirm = false;
      await load();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="page">
  {#if health}
    <div class="grid2">
      <div class="card stat"><b>{health.photos.toLocaleString('en')}</b><span class="muted small">photos in the library</span></div>
      <div class="card stat"><b>{health.devices_online} / {health.devices_total}</b><span class="muted small">devices online</span></div>
      <div class="card stat"><b>{health.originals_here.toLocaleString('en')}</b><span class="muted small">originals here · {bytes(health.originals_here_bytes)}</span></div>
    </div>

    {#if health.at_risk > 0}
      <Banner tone="risk"><b>{plural(health.at_risk, 'photo')}</b> {health.at_risk === 1 ? 'has' : 'have'} fewer than {health.redundancy_target} copies. They are copied automatically whenever another device is reachable.</Banner>
    {:else if health.photos > 0}
      <Banner tone="info" icon="✓">Every photo has at least {health.redundancy_target} copies.</Banner>
    {/if}

    <div class="section">Copies per photo <span class="muted">(target {health.redundancy_target})</span></div>
    <div class="card hist">
      {#each health.copies_histogram as n, i}
        {#if n > 0 || i <= health.redundancy_target}
          <div class="hrow">
            <span class="small">{bucketLabel(i, health.copies_histogram.length)}</span>
            <ProgressBar total={Math.max(1, health.photos)} segments={[{ value: n, tone: bucketTone(i, health.redundancy_target) }]} label={`${n} photos`} />
            <span class="small muted num">{n.toLocaleString('en')}</span>
          </div>
        {/if}
      {/each}
    </div>

    <div class="section">Free up space</div>
    <div class="card pad">
      {#if health.can_free_photos > 0}
        <p class="small">You can free <b>{bytes(health.can_free_bytes)}</b> by removing the originals of {plural(health.can_free_photos, 'photo')} from this device. Each one is safe on other devices that confirmed they have it; previews stay here.</p>
        <Button kind="primary" onclick={() => (confirm = true)}>Free up {bytes(health.can_free_bytes)}…</Button>
      {:else}
        <p class="small muted">Nothing can be freed right now. An original is only removed from this device when a keeper, or enough other devices, confirmed holding it in the last week, and never one you pinned. Keepers never free space.</p>
      {/if}
    </div>
  {:else}
    <span class="muted">Loading…</span>
  {/if}
</div>

<Sheet open={confirm} title="Free up space?" onclose={() => (confirm = false)} width={420}>
  {#if health}
    <div class="stack">
      <p class="small">The originals of {plural(health.can_free_photos, 'photo')} ({bytes(health.can_free_bytes)}) leave this device. You can still open them: the app fetches them from another device on demand.</p>
      <Button kind="primary" block disabled={busy} onclick={free}>{busy ? 'Freeing…' : 'Free up space'}</Button>
      <Button block onclick={() => (confirm = false)}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<style>
  .page { padding: 18px; max-width: 760px; overflow: auto; height: 100%; }
  .grid2 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; margin-bottom: 14px; }
  .stat { padding: 14px; display: flex; flex-direction: column; }
  .stat b { font-size: 22px; letter-spacing: -0.02em; }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .hist { padding: 12px 14px; display: grid; gap: 8px; }
  .hrow { display: grid; grid-template-columns: 90px 1fr 60px; gap: 10px; align-items: center; }
  .num { text-align: right; }
  .pad { padding: 14px; }
  p { margin: 0 0 12px; }
</style>
