<script lang="ts">
  // Devices and activity (mockups 5a/5b): who has the library, how they are connected, adding
  // a device with a QR code and comparing the six-digit code, and removing one.
  import { onMount, untrack } from 'svelte';
  import { api, type DeviceInfo, type DeviceStorageInfo, type PairStart } from '../lib/api';
  import { bytes, plural } from '../lib/format';
  import { pullRefresh } from '../lib/gestures';
  import { refreshEverything } from '../lib/refresh';
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
  let storage = $state<DeviceStorageInfo[]>([]);
  const used = (s: DeviceStorageInfo) => s.original_bytes + s.other_bytes;
  // The library's devices by how much they hold, largest first (bars are relative to it).
  const storageRows = $derived(
    devices
      .filter((d) => !d.revoked)
      .map((d) => ({ d, s: storage.find((x) => x.id === d.id) ?? { id: d.id, originals: 0, original_bytes: 0, other_bytes: 0 } }))
      .sort((a, b) => used(b.s) - used(a.s)),
  );
  const most = $derived(Math.max(1, ...storageRows.map((r) => used(r.s))));
  let addOpen = $state(false);
  let role = $state('phone');
  let pair = $state<PairStart | null>(null);
  let revoking = $state<DeviceInfo | null>(null);
  let phrase = $state('');
  let changing = $state<DeviceInfo | null>(null);
  let promoting = $state<DeviceInfo | null>(null);
  async function makeAdmin() {
    if (!promoting) return;
    busy = true;
    try {
      await api.makeAdmin(promoting.id, phrase);
      toast(`${promoting.is_this_device ? 'This device' : promoting.name} is now an admin`);
      promoting = null;
      phrase = '';
      await load();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
  let newRole = $state('phone');
  const roleHelp: Record<string, string> = {
    phone: 'Keeps previews of everything, and the originals of its own photos until they are backed up elsewhere.',
    keeper: 'Keeps every original: the backstop. Best for an always-on computer or a small box at home.',
    viewer: 'Sees previews only and cannot add or change photos. Good for a shared tablet or a TV.',
  };
  let busy = $state(false);
  const me = $derived(devices.find((d) => d.is_this_device));
  // Admins remove ordinary devices on their own; an admin device, or a removal from a device
  // that is no admin, needs the recovery phrase.
  const needsPhrase = $derived(!!revoking && (revoking.admin || !me?.admin));

  async function load() {
    devices = await api.devices();
    // Totals walk every photo's holdings: fetched after the list so it never waits on them.
    api.deviceStorage().then((s) => (storage = s)).catch(() => {});
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

  function openRole(d: DeviceInfo) {
    changing = d;
    newRole = d.role;
    phrase = '';
  }

  async function changeRole() {
    if (!changing) return;
    busy = true;
    try {
      await api.setDeviceRole(changing.id, newRole, me?.admin ? undefined : phrase);
      toast(`${changing.name} is now a ${newRole}`);
      changing = null;
      phrase = '';
      await load();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  async function revoke() {
    if (!revoking) return;
    try {
      await api.revokeDevice(revoking.id, needsPhrase ? phrase : undefined);
      toast(`${revoking.name} was removed from the library`);
      revoking = null;
      phrase = '';
      await load();
    } catch (e) {
      fail(e);
    }
  }

  const pathTone = (d: DeviceInfo) => (d.path === 'relay' ? 'warn' : d.path === 'none' ? 'mute' : 'ok');
  const pathLabel = (d: DeviceInfo) => ({ lan: 'Same network', direct: 'Direct', relay: 'Via relay', none: d.online ? 'Connecting' : 'Offline' })[d.path] ?? d.path;
</script>

<div class="page" use:pullRefresh={{ onRefresh: refreshEverything }}>
  <div class="row-flex head">
    <div class="grow muted small">Devices that hold this library. Everything stays between them.</div>
    <Button kind="primary" size="sm" onclick={() => { addOpen = true; pair = null; }}>＋ Add a device</Button>
  </div>

  {#if devices.length > 0 && devices.filter((d) => !d.revoked).length === 1}
    <div class="callout">
      <b>This is your only device</b>
      <span class="muted small">Photos on one device are one lost phone away from gone. Add a second device (a computer, or a small always-on keeper) and your library is backed up between them.</span>
      <div><Button kind="primary" size="sm" onclick={() => { addOpen = true; pair = null; }}>Add a device</Button></div>
    </div>
  {/if}

  <List>
    {#each devices.filter((d) => !d.revoked) as d (d.id)}
      <Row stack title={d.is_this_device ? `${d.name} (this device)` : d.name} subtitle={`${d.role}${d.admin ? ' · admin' : ''}${d.latency_ms != null ? ` · ${d.latency_ms} ms` : ''}`}>
        {#snippet lead()}<Avatar name={d.name} />{/snippet}
        {#snippet trail()}
          <span class="row-flex">
            {#if !d.is_this_device}<Pill tone={pathTone(d)}>{pathLabel(d)}</Pill>{/if}
            <span class="dot" class:on={d.online} class:relay={d.path === 'relay'} title={d.online ? 'online' : 'offline'}></span>
            {#if !d.admin}<Button size="sm" kind="ghost" onclick={() => { promoting = d; phrase = ''; }}>Make admin</Button>{/if}
            <Button size="sm" kind="ghost" onclick={() => openRole(d)}>Role</Button>
            {#if !d.is_this_device}<Button size="sm" kind="ghost" onclick={() => { revoking = d; phrase = ''; }}>Remove</Button>{/if}
          </span>
        {/snippet}
      </Row>
    {/each}
  </List>

  {#if storageRows.length > 0}
    <div class="section">Storage</div>
    <List>
      {#each storageRows as { d, s } (d.id)}
        <div class="store">
          <div class="row-flex between">
            <b>{d.is_this_device ? `${d.name} (this device)` : d.name}</b>
            <span>{bytes(used(s))}</span>
          </div>
          <div class="bar"><span class="orig" style:width="{(s.original_bytes / most) * 100}%"></span><span class="prev" style:width="{(s.other_bytes / most) * 100}%"></span></div>
          <div class="small muted">{s.originals ? `${plural(s.originals, 'original')} (${bytes(s.original_bytes)})` : 'No originals'} · previews {bytes(s.other_bytes)}</div>
        </div>
      {/each}
    </List>
    <p class="small muted">What each device keeps of this library, as this device last heard. Photos outside Shoebox are not counted.</p>
  {/if}

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

<Sheet open={!!promoting} title="Make an admin" onclose={() => { promoting = null; phrase = ''; }} width={400}>
  {#if promoting}
    <div class="stack">
      <p class="small">{promoting.is_this_device ? 'This device' : promoting.name} will be able to add devices, change roles and remove devices that are not admins, without the recovery phrase.</p>
      <p class="small muted">A second admin means losing one device does not leave every change needing the phrase. The other devices are told.</p>
      <textarea class="input" rows="3" placeholder="24-word recovery phrase" autocomplete="off" spellcheck="false" bind:value={phrase}></textarea>
      <Button kind="primary" block disabled={busy || !phrase.trim()} onclick={makeAdmin}>{busy ? 'Saving…' : 'Make admin'}</Button>
      <Button block onclick={() => { promoting = null; phrase = ''; }}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<Sheet open={!!changing} title="Change role" onclose={() => { changing = null; phrase = ''; }} width={400}>
  {#if changing}
    <div class="stack">
      <p class="small">What should {changing.is_this_device ? 'this device' : changing.name} do?</p>
      <Segmented bind:value={newRole} options={[{ value: 'phone', label: 'Phone' }, { value: 'keeper', label: 'Keeper' }, { value: 'viewer', label: 'Viewer' }]} />
      <p class="small muted">{roleHelp[newRole]}</p>
      {#if !me?.admin}
        <p class="small">This device is not an admin, so changing a role from here needs your recovery phrase.</p>
        <textarea class="input" rows="3" placeholder="24-word recovery phrase" autocomplete="off" spellcheck="false" bind:value={phrase}></textarea>
      {/if}
      <Button kind="primary" block disabled={busy || newRole === changing.role || (!me?.admin && !phrase.trim())} onclick={changeRole}>{busy ? 'Saving…' : 'Save'}</Button>
      <Button block onclick={() => { changing = null; phrase = ''; }}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<Sheet open={!!revoking} title="Remove this device?" onclose={() => { revoking = null; phrase = ''; }} width={400}>
  {#if revoking}
    <div class="stack">
      <p class="small">{revoking.name} will lose access to the library. Photos already on it stay there until someone deletes them, but it can no longer sync. Everyone else keeps working without interruption.</p>
      {#if needsPhrase}
        <p class="small">{revoking.admin ? `${revoking.name} is an admin device, so removing it` : 'This device is not an admin, so removing a device from here'} needs your recovery phrase. That way a stolen admin device cannot remove you instead.</p>
        <textarea class="input" rows="3" placeholder="24-word recovery phrase" autocomplete="off" spellcheck="false" bind:value={phrase}></textarea>
      {/if}
      <Button kind="danger" block disabled={needsPhrase && !phrase.trim()} onclick={revoke}>Remove {revoking.name}</Button>
      <Button block onclick={() => { revoking = null; phrase = ''; }}>Cancel</Button>
    </div>
  {/if}
</Sheet>

<style>
  .callout { display: flex; flex-direction: column; gap: 6px; padding: 14px; margin: 10px 0; border-radius: 12px; background: var(--accent-soft); }
  .page { padding: 18px; max-width: 760px; overflow: auto; height: 100%; }
  @media (max-width: 720px) { .page { padding: 12px 14px; } }
  .head { margin-bottom: 14px; }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--ink-3); display: inline-block; }
  .dot.on { background: var(--ok); }
  .dot.relay { background: var(--warn); }
  .input { width: 100%; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px 12px; color: var(--ink); font: inherit; resize: none; }
  .ticket { width: 100%; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); padding: 8px; resize: none; }
  .code { display: flex; gap: 8px; justify-content: center; font: 700 30px var(--mono); margin: 12px 0 18px; }
  .code span { width: 40px; height: 52px; background: var(--surface-2); border-radius: 10px; display: grid; place-items: center; }
  .err { color: var(--risk); }
  .store { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border-bottom: 1px solid var(--line); }
  .store:last-child { border-bottom: 0; }
  .between { justify-content: space-between; }
  .bar { display: flex; height: 6px; border-radius: 3px; background: var(--surface-2); overflow: hidden; }
  .bar .orig { background: var(--accent); }
  .bar .prev { background: var(--ink-3); opacity: .5; }
</style>
