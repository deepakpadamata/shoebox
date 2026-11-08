<script lang="ts">
  // Add a device (U1-13): the QR code first. The new device's role is guessed from this one (a
  // phone is usually paired with a computer that keeps everything, a computer with a phone) and
  // changes in one row. The six-digit check afterwards is unchanged.
  import { untrack } from 'svelte';
  import { api, type PairStart } from '../lib/api';
  import { isHandheld } from '../lib/pick';
  import { ROLE_DOES, ROLE_HELP, roleLabel } from '../lib/roles';
  import { app, fail, toast } from '../lib/store.svelte';
  import Button from './Button.svelte';
  import QR from './QR.svelte';
  import Segmented from './Segmented.svelte';
  import Sheet from './Sheet.svelte';

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  let role = $state(isHandheld() ? 'keeper' : 'companion');
  let pair = $state<PairStart | null>(null);
  let changing = $state(false);
  let busy = $state(false);

  async function start() {
    busy = true;
    app.pairingError = '';
    try {
      pair = await api.pairStart(role);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  // Opening the sheet shows the code at once.
  $effect(() => {
    if (open) untrack(() => { pair = null; changing = false; start(); });
  });
  $effect(() => {
    app.addDeviceOpen = open;
    return () => { app.addDeviceOpen = false; };
  });

  // Closed by itself once the pairing finished (the list refreshes through the event).
  let seen = app.pairVersion;
  $effect(() => {
    const v = app.pairVersion;
    untrack(() => {
      if (open && v > seen) { toast('Device added'); close(); }
      seen = v;
    });
  });

  function close() {
    pair = null;
    app.pairing = null;
    onclose();
  }
  async function answer(accept: boolean) {
    await api.confirmPairing(accept).catch(fail);
    if (!accept) { app.pairing = null; pair = null; start(); }
  }
  async function pickRole(r: string) {
    role = r;
    pair = null;
    await start();
  }
  const sendLink = () => api.shareText(pair!.ticket).catch((e) => toast(e instanceof Error ? e.message : String(e)));
</script>

<Sheet {open} title="Add a device" onclose={close} width={440}>
  {#if app.pairing}
    <p class="muted small">Does <b>{app.pairing.peer_name}</b> show the same six digits?</p>
    <div class="code" aria-label="Pairing code">{#each app.pairing.sas.split('') as d}<span>{d}</span>{/each}</div>
    <div class="stack">
      <Button kind="primary" block onclick={() => answer(true)}>They match</Button>
      <Button block onclick={() => answer(false)}>They do not match</Button>
    </div>
  {:else}
    <div class="center stack">
      {#if pair}
        <QR svg={pair.qr_svg} />
        <p class="small muted">Open Shoebox on the new device and scan this. It works once and expires in five minutes.</p>
        <div class="two">
          <Button kind="primary" onclick={sendLink}>Send pairing link instead</Button>
          <Button onclick={() => navigator.clipboard.writeText(pair!.ticket).then(() => toast('Copied'))}>Copy code</Button>
        </div>
      {:else}
        <p class="muted small">Making a pairing code…</p>
      {/if}
      {#if app.pairingError}<div class="err small">{app.pairingError}</div>{/if}
    </div>
    <div class="role">
      <div class="small"><span class="muted">The new device</span> <b>{ROLE_DOES[role]}</b> <span class="tag">{roleLabel(role)}</span></div>
      {#if changing}
        <Segmented value={role} options={[{ value: 'companion', label: 'Companion' }, { value: 'keeper', label: 'Keeper' }, { value: 'viewer', label: 'Viewer' }]} onchange={pickRole} />
        <p class="small muted">{ROLE_HELP[role]}</p>
      {:else}
        <button class="link" onclick={() => (changing = true)}>Change</button>
      {/if}
    </div>
  {/if}
</Sheet>

<style>
  .code { display: flex; gap: 8px; justify-content: center; font: 700 30px var(--mono); margin: 12px 0 18px; }
  .code span { width: 40px; height: 52px; background: var(--surface-2); border-radius: 10px; display: grid; place-items: center; }
  .two { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
  .role { margin-top: 16px; padding-top: 12px; border-top: 1px solid var(--line); display: grid; gap: 8px; }
  .tag { font-size: var(--t-xs); padding: 2px 8px; border-radius: 99px; background: var(--surface-2); color: var(--ink-2); font-weight: 600; }
  .link { justify-self: start; border: 0; background: none; color: var(--accent); font: inherit; font-weight: 600; cursor: pointer; padding: 4px 0; }
  .err { color: var(--risk); }
</style>
