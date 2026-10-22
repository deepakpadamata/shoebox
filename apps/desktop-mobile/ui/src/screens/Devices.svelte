<script lang="ts">
  // Devices and activity (mockups 5a/5b): who has the library, how they are connected, adding
  // a device with a QR code and comparing the six-digit code, and removing one.
  import { onMount, untrack } from 'svelte';
  import { api, type DeviceInfo, type PairStart } from '../lib/api';
  import { app, fail, toast } from '../lib/store.svelte';
  import Avatar from '../components/Avatar.svelte';
  import Button from '../components/Button.svelte';
  import List from '../components/List.svelte';
  import Pill from '../components/Pill.svelte';
  import QR from '../components/QR.svelte';
  import Row from '../components/Row.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Sheet from '../components/Sheet.svelte';

  let devices = $state<DeviceInfo[]>([]);
  let addOpen = $state(false);
  let role = $state('phone');
  let pair = $state<PairStart | null>(null);
  let revoking = $state<DeviceInfo | null>(null);
  let busy = $state(false);
  const me = $derived(devices.find((d) => d.is_this_device));

  async function load() {
    devices = await api.devices();
  }
  onMount(() => { load().catch(fail); });
  $effect(() => {
    app.devicesVersion;
    untrack(() => { load().catch(() => {}); });
  });

  async function startPairing() {
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

  async function answer(accept: boolean) {
    await api.confirmPairing(accept).catch(fail);
    if (!accept) { app.pairing = null; pair = null; }
  }

  // The sheet closes itself once the pairing finished (the list refreshes through the event).
  let pairSeen = app.pairVersion;
  $effect(() => {
    const v = app.pairVersion;
    untrack(() => {
      if (addOpen && v > pairSeen) {
        toast('Device added');
        closeAdd();
      }
      pairSeen = v;
    });
  });

  function closeAdd() {
    addOpen = false;
    pair = null;
    app.pairing = null;
  }

  async function revoke() {
    if (!revoking) return;
    try {
      await api.revokeDevice(revoking.id);
      toast(`${revoking.name} was removed from the library`);
      revoking = null;
      await load();
    } catch (e) {
      fail(e);
    }
  }

  const pathTone = (d: DeviceInfo) => (d.path === 'relay' ? 'warn' : d.path === 'none' ? 'mute' : 'ok');
  const pathLabel = (d: DeviceInfo) => ({ lan: 'Same network', direct: 'Direct', relay: 'Via relay', none: d.online ? 'Connecting' : 'Offline' })[d.path] ?? d.path;
</script>

<div class="page">
  <div class="row-flex head">
    <div class="grow muted small">Devices that hold this library. Everything stays between them.</div>
    <Button kind="primary" size="sm" onclick={() => { addOpen = true; pair = null; }}>＋ Add a device</Button>
  </div>

  <List>
    {#each devices.filter((d) => !d.revoked) as d (d.id)}
      <Row title={d.is_this_device ? `${d.name} (this device)` : d.name} subtitle={`${d.role}${d.admin ? ' · admin' : ''}${d.latency_ms != null ? ` · ${d.latency_ms} ms` : ''}`}>
        {#snippet lead()}<Avatar name={d.name} />{/snippet}
        {#snippet trail()}
          <span class="row-flex">
            {#if !d.is_this_device}<Pill tone={pathTone(d)}>{pathLabel(d)}</Pill>{/if}
            <span class="dot" class:on={d.online} class:relay={d.path === 'relay'} title={d.online ? 'online' : 'offline'}></span>
            {#if !d.is_this_device && me?.admin}<Button size="sm" kind="ghost" onclick={() => (revoking = d)}>Remove</Button>{/if}
          </span>
        {/snippet}
      </Row>
    {/each}
  </List>

  {#if devices.some((d) => d.revoked)}
    <div class="section">Removed</div>
    <List>
      {#each devices.filter((d) => d.revoked) as d (d.id)}
        <Row title={d.name} subtitle="Removed from the library">
          {#snippet lead()}<Avatar name={d.name} hue={0} />{/snippet}
        </Row>
      {/each}
    </List>
  {/if}
</div>

<Sheet open={addOpen} title="Add a device" onclose={closeAdd} width={440}>
  {#if app.pairing}
    <p class="muted small">Does <b>{app.pairing.peer_name}</b> show the same six digits?</p>
    <div class="code" aria-label="Pairing code">{#each app.pairing.sas.split('') as d}<span>{d}</span>{/each}</div>
    <div class="stack">
      <Button kind="primary" block onclick={() => answer(true)}>They match</Button>
      <Button block onclick={() => answer(false)}>They do not match</Button>
    </div>
  {:else if pair}
    <div class="center stack">
      <QR svg={pair.qr_svg} />
      <p class="small muted">Scan this on the new device, or paste the code below. It works once and expires in five minutes.</p>
      <textarea class="ticket mono" readonly rows="3" onclick={(e) => e.currentTarget.select()}>{pair.ticket}</textarea>
      <Button size="sm" onclick={() => navigator.clipboard.writeText(pair!.ticket).then(() => toast('Copied'))}>Copy code</Button>
      {#if app.pairingError}<div class="err small">{app.pairingError}</div>{/if}
    </div>
  {:else}
    <div class="stack">
      <p class="muted small">What will the new device do?</p>
      <Segmented bind:value={role} options={[{ value: 'phone', label: 'Phone' }, { value: 'keeper', label: 'Keeper' }, { value: 'viewer', label: 'Viewer' }]} />
      <Button kind="primary" block disabled={busy} onclick={startPairing}>Show pairing code</Button>
    </div>
  {/if}
</Sheet>

<Sheet open={!!revoking} title="Remove this device?" onclose={() => (revoking = null)} width={400}>
  {#if revoking}
    <div class="stack">
      <p class="small">{revoking.name} will lose access to the library. Photos already on it stay there until someone deletes them, but it can no longer sync. Everyone else keeps working without interruption.</p>
      <Button kind="danger" block onclick={revoke}>Remove {revoking.name}</Button>
      <Button block onclick={() => (revoking = null)}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<style>
  .page { padding: 18px; max-width: 760px; overflow: auto; height: 100%; }
  .head { margin-bottom: 14px; }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--ink-3); display: inline-block; }
  .dot.on { background: var(--ok); }
  .dot.relay { background: var(--warn); }
  .ticket { width: 100%; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); padding: 8px; resize: none; }
  .code { display: flex; gap: 8px; justify-content: center; font: 700 30px var(--mono); margin: 12px 0 18px; }
  .code span { width: 40px; height: 52px; background: var(--surface-2); border-radius: 10px; display: grid; place-items: center; }
  .err { color: var(--risk); }
</style>
