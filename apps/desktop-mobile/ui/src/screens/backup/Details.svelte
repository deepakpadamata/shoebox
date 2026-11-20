<script lang="ts">
  // Backup details (U1-12): the copies chart, what is on this device, and Free up space. The
  // sentence above all this comes from `app.backup_status`; this page only shows how it is made up.
  import { onMount, untrack } from 'svelte';
  import { api, type FreeUpReport, type Health } from '../../lib/api';
  import { bytes, plural } from '../../lib/format';
  import { app, fail, toast } from '../../lib/store.svelte';
  import Button from '../../components/Button.svelte';
  import ProgressBar from '../../components/ProgressBar.svelte';
  import Sheet from '../../components/Sheet.svelte';

  let health = $state<Health | null>(null);
  let confirm = $state(false);
  let busy = $state(false);

  // `health` is heavy (it walks the whole library and asks other devices what they hold), and
  // the library changes many times a second while photos are freed or fetched: one call at a
  // time, with at most one more queued, and none while a Free up run is going on (it ends with
  // a refresh of its own).
  let loading: Promise<void> | null = null;
  let again = false;
  function load(): Promise<void> {
    if (loading) { again = true; return loading; }
    loading = (async () => {
      try {
        do { again = false; health = await api.health(); } while (again);
      } finally {
        loading = null;
      }
    })();
    return loading;
  }
  const freeing = $derived(app.freeUp?.state === 'running');
  onMount(() => { load().catch(fail); });
  let timer: ReturnType<typeof setTimeout> | undefined;
  let mounted = false;
  $effect(() => {
    app.version;
    app.holdingsVersion;
    if (freeing) return;
    if (!mounted) { mounted = true; return; } // onMount loads the first time
    untrack(() => {
      clearTimeout(timer);
      timer = setTimeout(() => load().catch(() => {}), 300);
    });
    return () => clearTimeout(timer);
  });

  // Photos below the target are listed by their copies; every photo that is safe under the rule
  // (enough copies, or a keeper holds it) is one row, as the core counts them.
  const rows = $derived.by(() => {
    if (!health) return [];
    const t = Math.max(1, health.redundancy_target);
    const h = health.copies_histogram;
    const out: { label: string; n: number; tone: 'risk' | 'warn' | 'ok' }[] = [];
    for (let i = 0; i < Math.min(t, h.length); i++) {
      if (h[i] > 0) out.push({ label: i === 0 ? 'No copy' : plural(i, 'copy', 'copies'), n: h[i], tone: i === 0 ? 'risk' : 'warn' });
    }
    out.push({ label: 'Backed up', n: h.slice(t).reduce((a, b) => a + b, 0), tone: 'ok' });
    return out;
  });

  // What a run would do, asked when the sheet opens: the gallery files among them.
  let plan = $state<FreeUpReport | null>(null);
  $effect(() => {
    if (!confirm) return;
    plan = null;
    api.freeUpSpace(true).then((r) => { if (confirm) plan = r; }).catch(() => {});
  });

  // The run goes on in the background: the sheet closes at once and the app stays usable. Its
  // progress shows here and its result as a toast, whichever screen is open by then.
  async function free() {
    busy = true;
    const before = app.freeUp;
    app.freeUp = { state: 'running', done: 0, total: plan?.photos ?? 0, report: null, error: null };
    try {
      const s = await api.startFreeUp();
      if (s.state !== 'running') app.freeUp = s;
      confirm = false;
      toast('Freeing up space in the background');
    } catch (e) {
      app.freeUp = before;
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="details">
  {#if health}
    <div class="grid2">
      <div class="card stat"><b>{health.photos.toLocaleString('en')}</b><span class="muted small">photos in the library</span></div>
      <div class="card stat"><b>{health.devices_online} / {health.devices_total}</b><span class="muted small">devices online</span></div>
      <div class="card stat"><b>{health.originals_here.toLocaleString('en')}</b><span class="muted small">originals here · {bytes(health.originals_here_bytes)}</span></div>
    </div>

    <div class="section">Copies <span class="muted">(safe: {plural(health.redundancy_target, 'copy', 'copies')} on devices, or one on a keeper)</span></div>
    <div class="card hist">
      {#each rows as r}
        <div class="hrow">
          <span class="small">{r.label}</span>
          <ProgressBar total={Math.max(1, health.photos)} segments={[{ value: r.n, tone: r.tone }]} label={`${r.n} photos`} />
          <span class="small muted num">{r.n.toLocaleString('en')}</span>
        </div>
      {/each}
    </div>

    {#if freeing && app.freeUp}
      <div class="section">Free up space</div>
      <div class="card pad">
        <p class="small">Freeing up space… {#if app.freeUp.total > 0}{app.freeUp.done.toLocaleString('en')} of {plural(app.freeUp.total, 'photo')}{/if}</p>
        <ProgressBar total={Math.max(1, app.freeUp.total)} segments={[{ value: app.freeUp.done, tone: 'ok' }]} label={`${app.freeUp.done} of ${app.freeUp.total} photos`} />
        <p class="small muted">This goes on in the background: you can keep using Shoebox.</p>
      </div>
    {:else if health.can_free_photos > 0}
      <div class="section">Free up space</div>
      <div class="card pad">
        <p class="small">You can free <b>{bytes(health.can_free_bytes)}</b> by removing the originals of {plural(health.can_free_photos, 'photo')} from this device. Each one is safe on other devices that confirmed they have it; previews stay here.</p>
        <Button kind="primary" onclick={() => (confirm = true)}>Free up {bytes(health.can_free_bytes)}…</Button>
      </div>
    {/if}
  {:else}
    <span class="muted">Loading…</span>
  {/if}
</div>

<Sheet open={confirm} title="Free up space?" onclose={() => (confirm = false)} width={420}>
  {#if health}
    <div class="stack">
      <p class="small">The originals of {plural(plan?.photos ?? health.can_free_photos, 'photo')} ({bytes(plan?.bytes ?? health.can_free_bytes)}) leave this device. You can still open them: the app fetches them from another device on demand.</p>
      {#if plan && plan.gallery > 0}
        <p class="small"><b>{plural(plan.gallery, 'photo')} {plan.gallery === 1 ? 'is' : 'are'} also deleted from this phone's gallery</b> (they stay in your library). Android asks you to allow it next.</p>
      {/if}
      <Button kind="primary" block disabled={busy || freeing} onclick={free}>Free up space</Button>
      <Button block onclick={() => (confirm = false)}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<style>
  .grid2 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; margin-bottom: 14px; }
  .stat { padding: 14px; display: flex; flex-direction: column; }
  .stat b { font-size: 22px; letter-spacing: -0.02em; }
  .section { font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .hist { padding: 12px 14px; display: grid; gap: 8px; }
  .hrow { display: grid; grid-template-columns: 90px 1fr 60px; gap: 10px; align-items: center; }
  .num { text-align: right; }
  .pad { padding: 14px; }
  p { margin: 0 0 12px; }
</style>
