<script lang="ts">
  // "Free up on this device" for chosen photos (the opposite of Keep on this device). Shows what a
  // run would do first, then does it. Gallery files are deleted from the gallery (Android asks
  // first); the photos stay in the library either way.
  import Sheet from './Sheet.svelte';
  import Button from './Button.svelte';
  import { api, type FreeUpReport } from '../lib/api';
  import { bytes, plural } from '../lib/format';
  import { app, fail, toast } from '../lib/store.svelte';
  import { isHandheld } from '../lib/pick';

  let { ids, open, onclose, ondone }: { ids: string[]; open: boolean; onclose: () => void; ondone?: () => void } = $props();
  const here = isHandheld() ? 'this phone' : 'this device';
  let plan = $state<FreeUpReport | null>(null);
  let busy = $state(false);

  $effect(() => {
    if (!open) return;
    plan = null;
    const chosen = ids;
    api.freePhotos(chosen, true).then((r) => { if (open) plan = r; }).catch((e) => { fail(e); onclose(); });
  });

  // The run goes on in the background (as Free up space does): the sheet closes at once, and the
  // result comes as a toast whichever screen is open by then.
  async function free() {
    busy = true;
    const before = app.freeUp;
    app.freeUp = { state: 'running', done: 0, total: plan?.photos ?? 0, report: null, error: null };
    try {
      const s = await api.startFreeUp(ids);
      if (s.state !== 'running') app.freeUp = s;
      if (ids.length > 1) toast('Freeing up space in the background');
      onclose();
      ondone?.();
    } catch (e) {
      app.freeUp = before;
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<Sheet {open} title="Free up on {here}?" {onclose} width={420}>
  <div class="stack">
    {#if !plan}
      <p class="small muted">Checking which devices have {ids.length === 1 ? 'this photo' : 'these photos'}…</p>
    {:else if plan.photos === 0}
      <p class="small">
        {ids.length === 1 ? 'This photo stays' : 'These photos stay'} on {here}: no other device has confirmed it holds {ids.length === 1 ? 'it' : 'them'}, a shared album needs {ids.length === 1 ? 'it' : 'them'} here, or {here} has no full-size copy.
      </p>
      <Button block onclick={onclose}>Close</Button>
    {:else}
      <p class="small">
        {#if plan.photos === 1}
          The full-size copy of 1 photo ({bytes(plan.bytes)}) leaves {here}. The photo stays in your library and on your other devices, and opens as before: the app fetches it when you look.
        {:else}
          The full-size copies of {plural(plan.photos, 'photo')} ({bytes(plan.bytes)}) leave {here}. The photos stay in your library and on your other devices, and open as before: the app fetches them when you look.
        {/if}
      </p>
      {#if plan.gallery > 0}
        <p class="small"><b>{plural(plan.gallery, 'photo')} {plan.gallery === 1 ? 'is' : 'are'} also deleted from this phone's gallery.</b> Android asks you to allow it next.</p>
      {/if}
      {#if plan.kept > 0}
        <p class="small muted">{plural(plan.kept, 'other photo')} {plan.kept === 1 ? 'stays as it is' : 'stay as they are'}: not here in full size, not yet confirmed on another device, or needed here by a shared album.</p>
      {/if}
      <Button kind="primary" block disabled={busy || app.freeUp?.state === 'running'} onclick={free}>{app.freeUp?.state === 'running' ? 'Another Free up is running…' : `Free up ${bytes(plan.bytes)}`}</Button>
      <Button block onclick={onclose}>Cancel</Button>
    {/if}
  </div>
</Sheet>
