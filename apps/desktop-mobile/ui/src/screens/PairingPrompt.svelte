<script lang="ts">
  // The six-digit code of a pairing this device offered, wherever the person is in the app: the
  // ticket is often copied out of the "Add a device" sheet and the sheet closed, and a code with
  // nowhere to show left the other device waiting until the pairing timed out.
  import { api } from '../lib/api';
  import { untrack } from 'svelte';
  import { app, fail, toast } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';
  import Sheet from '../components/Sheet.svelte';

  const shown = $derived(!!app.pairing && !app.addDeviceOpen);

  // Shown here: say so when it went through (the Devices sheet does that for its own).
  let seen = app.pairVersion;
  let asked = false;
  $effect(() => {
    if (shown) asked = true;
    const v = app.pairVersion;
    untrack(() => {
      if (v > seen && asked) toast('Device added');
      if (v > seen) asked = false;
      seen = v;
    });
  });

  async function answer(accept: boolean) {
    await api.confirmPairing(accept).catch(fail);
    if (!accept) app.pairing = null;
  }
</script>

<Sheet open={shown} title="Add a device" onclose={() => answer(false)} width={440}>
  {#if app.pairing}
    <p class="muted small">Does <b>{app.pairing.peer_name}</b> show the same six digits?</p>
    <div class="code" aria-label="Pairing code">{#each app.pairing.sas.split('') as d}<span>{d}</span>{/each}</div>
    <div class="stack">
      <Button kind="primary" block onclick={() => answer(true)}>They match</Button>
      <Button block onclick={() => answer(false)}>They do not match</Button>
    </div>
  {/if}
</Sheet>

<style>
  .code { display: flex; gap: 8px; justify-content: center; font: 700 30px var(--mono); margin: 12px 0 18px; }
  .code span { width: 40px; height: 52px; background: var(--surface-2); border-radius: 10px; display: grid; place-items: center; }
</style>
