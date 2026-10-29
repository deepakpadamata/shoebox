<script lang="ts">
  // Devices (mockup 07): one list of who has the library, how each is connected, what it keeps and
  // what it is sending or getting right now, under a card that says what is moving and opens into
  // this device's queue. Adding a device with a QR code and the six-digit code, roles, removing.
  import { onDestroy, onMount, untrack } from 'svelte';
  import { api, blobUrl, type DeviceInfo, type DeviceStorageInfo, type PairStart, type TransferItem, type Transfers } from '../lib/api';
  import { GrowthRates, rowActivity, speed, summarize } from '../lib/activity';
  import { bytes, plural } from '../lib/format';
  import { viewport } from '../lib/viewport.svelte';
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
  let transfers = $state<Transfers | null>(null);
  let total = $state(0);
  let rates = $state<Record<string, number>>({});
  const growth = new GrowthRates();
  const used = (s: DeviceStorageInfo) => s.original_bytes + s.other_bytes;
  const storeOf = (d: DeviceInfo): DeviceStorageInfo => storage.find((x) => x.id === d.id) ?? { id: d.id, originals: 0, original_bytes: 0, other_bytes: 0 };
  // This device first, then the others by how much they keep (the bars share one scale).
  const rows = $derived(
    devices
      .filter((d) => !d.revoked)
      .map((d) => ({ d, s: storeOf(d), act: rowActivity(d, transfers, devices, storage, total, rates) }))
      .sort((a, b) => Number(b.d.is_this_device) - Number(a.d.is_this_device) || used(b.s) - used(a.s)),
  );
  const most = $derived(Math.max(1, ...rows.map((r) => used(r.s))));
  const summary = $derived(summarize(transfers, devices, storage, total, rates));
  const moving = $derived<Array<{ t: TransferItem; dir: 'down' | 'up' }>>(
    transfers ? [...transfers.downloading.map((t) => ({ t, dir: 'down' as const })), ...transfers.sending.map((t) => ({ t, dir: 'up' as const }))] : [],
  );
  let queueOpen = $state(false);
  let menuFor = $state<string | null>(null);
  let detail = $state<DeviceInfo | null>(null);
  const nameOf = (id: string | null) => devices.find((d) => d.id === id)?.name ?? 'another device';
  const kindLabel: Record<string, string> = { thumb: 'Thumbnail', preview: 'Preview', original: 'Original', other: 'Library data' };
  let addOpen = $state(false);
  let role = $state('companion');
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
  let newRole = $state('companion');
  const roleHelp: Record<string, string> = {
    companion: 'Keeps previews of everything and the full-size photos you pin, favourite or took recently; the rest lives on your keepers. For phones, tablets and laptops.',
    keeper: 'Keeps every original: the backstop. Best for an always-on computer or a small box at home.',
    viewer: 'Sees previews only and cannot add or change photos. Good for a shared tablet or a TV.',
  };
  let busy = $state(false);
  const me = $derived(devices.find((d) => d.is_this_device));
  // Admins remove ordinary devices on their own; an admin device, or a removal from a device
  // that is no admin, needs the recovery phrase.
  const needsPhrase = $derived(!!revoking && (revoking.admin || !me?.admin));

  async function loadStorage() {
    const [s, c] = await Promise.all([api.deviceStorage(), api.counts()]);
    storage = s;
    total = c.photos;
    growth.add(s);
    rates = growth.rates();
  }
  async function load() {
    devices = await api.devices();
    // Totals walk every photo's holdings: fetched after the list so it never waits on them.
    loadStorage().catch(() => {});
  }
  // While the screen is open: what moves, every 1.5 s; who holds what, every 10 s.
  let ticks = 0;
  const poll = setInterval(() => {
    api.transfers().then((t) => (transfers = t)).catch(() => {});
    if (++ticks % 7 === 0) loadStorage().catch(() => {});
  }, 1500);
  onDestroy(() => clearInterval(poll));
  onMount(() => {
    load().catch(fail);
    api.transfers().then((t) => (transfers = t)).catch(() => {});
  });
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

  $effect(() => {
    app.addDeviceOpen = addOpen;
    return () => { app.addDeviceOpen = false; };
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

  // An offline device keeps its last path in the list: say offline, not how it was connected.
  const pathTone = (d: DeviceInfo) => (!d.online || d.path === 'none' ? 'mute' : d.path === 'relay' ? 'warn' : 'ok');
  const pathLabel = (d: DeviceInfo) => (!d.online ? 'Offline' : ({ lan: 'Same network', direct: 'Direct', relay: 'Via relay', none: 'Connecting' })[d.path] ?? d.path);
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

  {#if transfers}
    <div class="card activity {summary.tone}">
      <div class="headline">
        <span class="spinner" aria-hidden="true"></span>
        <b class="grow">{summary.head}</b>
        {#if summary.tone !== 'idle' && !viewport.mobile}<Button size="sm" kind="ghost" onclick={() => (queueOpen = !queueOpen)}>{queueOpen ? 'Hide queue' : 'Show queue'}</Button>{/if}
      </div>
      {#if summary.progress != null}<div class="prog"><span style:width="{summary.progress * 100}%"></span></div>{/if}
      {#if summary.sub}<div class="small muted">{summary.sub}</div>{/if}
      {#if queueOpen && summary.tone !== 'idle'}
        <div class="queue">
          <div class="counts small muted">
            <span>Thumbnails <b>{transfers.waiting_thumbs.toLocaleString('en')}</b></span>
            <span>Previews <b>{transfers.waiting_previews.toLocaleString('en')}</b></span>
            <span>Originals <b>{transfers.waiting_originals.toLocaleString('en')}</b> waiting{transfers.waiting_bytes ? ` (${bytes(transfers.waiting_bytes)})` : ''}</span>
            {#if transfers.sending.length}<span>Sending <b>{transfers.sending.length}</b></span>{/if}
          </div>
          {#each moving.slice(0, 8) as { t, dir } (dir + t.photo + t.kind + t.peer)}
            <div class="qrow">
              {#if t.photo}<img class="qthumb" src={blobUrl(t.photo, 'thumb')} alt="" />{:else}<span class="qthumb"></span>{/if}
              <div class="grow">
                <div class="small">{kindLabel[t.kind] ?? t.kind} <span class="muted">· {dir === 'down' ? `from ${nameOf(t.peer)}` : `to ${nameOf(t.peer)}`}</span></div>
                <div class="prog thin" class:up={dir === 'up'}><span style:width="{t.size ? Math.min(100, (t.done / t.size) * 100) : 0}%"></span></div>
              </div>
              <span class="tiny muted mono">{bytes(t.done)}{t.size ? ` / ${bytes(t.size)}` : ''}</span>
            </div>
          {/each}
          {#if moving.length > 8}<div class="tiny muted">and {plural(moving.length - 8, 'more')}</div>{/if}
          {#if moving.length === 0}<div class="tiny muted">Nothing in flight right now.</div>{/if}
        </div>
      {/if}
      {#if summary.tone !== 'idle' && viewport.mobile}<div><Button size="sm" kind="ghost" onclick={() => (queueOpen = !queueOpen)}>{queueOpen ? 'Hide queue' : 'Show queue'}</Button></div>{/if}
    </div>
  {/if}

  {#if viewport.mobile}
    <List>
      {#each rows as { d, s, act } (d.id)}
        <button class="prow" onclick={() => (detail = d)}>
          <div class="row-flex">
            <Avatar name={d.name} size={38} />
            <div class="grow left">
              <div class="t1">{d.name}{#if d.is_this_device}<span class="muted">&nbsp;(this device)</span>{/if}</div>
              <div class="t2 muted">{d.role}{d.admin ? ' · admin' : ''}</div>
            </div>
            {#if !d.is_this_device}<Pill tone={pathTone(d)}>{pathLabel(d)}</Pill>{/if}
            <span class="dot" class:on={d.online} class:relay={d.path === 'relay'}></span>
            <span class="chev" aria-hidden="true">›</span>
          </div>
          <div class="bar"><span class="orig" style:width="{(s.original_bytes / most) * 100}%"></span><span class="prev" style:width="{(s.other_bytes / most) * 100}%"></span></div>
          <div class="tiny muted between row-flex"><span>{s.originals ? plural(s.originals, 'original') : 'No originals'} · previews {bytes(s.other_bytes)}</span><b class="ink">{bytes(used(s))}</b></div>
          {#if act.text && act.tone !== 'idle'}<div class="small act {act.tone}">{act.text}{#if act.detail} <span class="muted">· {act.detail}</span>{/if}</div>{/if}
        </button>
      {/each}
    </List>
    <p class="tiny muted">Orange: originals; grey: previews and thumbnails. As this device last heard; photos outside Shoebox are not counted.</p>
  {:else}
    <div class="table">
      <div class="thead"><span>Device</span><span>Connection</span><span>Keeps</span><span>Right now</span><span></span></div>
      {#each rows as { d, s, act } (d.id)}
        <div class="trow">
          <div class="row-flex who">
            <Avatar name={d.name} size={36} />
            <div class="left"><div class="t1">{d.name}{#if d.is_this_device}<span class="muted">&nbsp;(this device)</span>{/if}</div><div class="t2 muted">{d.role}{d.admin ? ' · admin' : ''}</div></div>
          </div>
          <div class="row-flex conn">
            {#if d.is_this_device}<Pill>This device</Pill>{:else}<Pill tone={pathTone(d)}>{pathLabel(d)}</Pill><span class="dot" class:on={d.online} class:relay={d.path === 'relay'}></span>{#if d.latency_ms != null}<span class="tiny muted">{d.latency_ms} ms</span>{/if}{/if}
          </div>
          <div class="keeps">
            <div class="bar"><span class="orig" style:width="{(s.original_bytes / most) * 100}%"></span><span class="prev" style:width="{(s.other_bytes / most) * 100}%"></span></div>
            <div class="row-flex between small"><span class="muted">{s.originals ? `${plural(s.originals, 'original')} · ${bytes(s.original_bytes)}` : 'No originals'} · previews {bytes(s.other_bytes)}</span><b>{bytes(used(s))}</b></div>
          </div>
          <div class="small act {act.tone}">{act.text}{#if act.detail}<div class="tiny muted">{act.detail}</div>{/if}</div>
          <div class="menu-wrap">
            <button class="kebab" aria-label="Actions for {d.name}" onclick={(e) => { e.stopPropagation(); menuFor = menuFor === d.id ? null : d.id; }}>⋯</button>
            {#if menuFor === d.id}
              <div class="menu" role="menu">
                {#if !d.admin}<button role="menuitem" onclick={() => { menuFor = null; promoting = d; phrase = ''; }}>Make admin…</button>{/if}
                <button role="menuitem" onclick={() => { menuFor = null; openRole(d); }}>Change role…</button>
                {#if !d.is_this_device}<button role="menuitem" class="danger" onclick={() => { menuFor = null; revoking = d; phrase = ''; }}>Remove…</button>{/if}
              </div>
            {/if}
          </div>
        </div>
      {/each}
    </div>
    <p class="tiny muted">Bars share one scale. Orange: originals; grey: previews and thumbnails. As this device last heard; photos outside Shoebox are not counted.</p>
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

<svelte:window onclick={() => (menuFor = null)} />

<Sheet open={!!detail} title={detail ? (detail.is_this_device ? `${detail.name} (this device)` : detail.name) : ''} onclose={() => (detail = null)} width={420}>
  {#if detail}
    {@const d = detail}
    {@const s = storeOf(d)}
    {@const act = rowActivity(d, transfers, devices, storage, total, rates)}
    <div class="stack">
      <dl class="kv small">
        <dt>Role</dt><dd>{d.role}{d.admin ? ' · admin' : ''}</dd>
        <dt>Connection</dt><dd>{d.is_this_device ? 'This device' : `${pathLabel(d)}${d.latency_ms != null ? ` · ${d.latency_ms} ms` : ''}`}</dd>
        <dt>Originals</dt><dd>{s.originals.toLocaleString('en')} · {bytes(s.original_bytes)}</dd>
        <dt>Previews and thumbnails</dt><dd>{bytes(s.other_bytes)}</dd>
        {#if act.text}<dt>Right now</dt><dd>{act.text}{act.detail ? ` · ${act.detail}` : ''}</dd>{/if}
      </dl>
      <div class="acts">
        {#if !d.admin}<Button kind="ghost" block onclick={() => { detail = null; promoting = d; phrase = ''; }}>Make admin…</Button>{/if}
        <Button kind="ghost" block onclick={() => { detail = null; openRole(d); }}>Change role…</Button>
        {#if !d.is_this_device}<Button kind="danger" block onclick={() => { detail = null; revoking = d; phrase = ''; }}>Remove from library…</Button>{/if}
      </div>
    </div>
  {/if}
</Sheet>

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
      <Segmented bind:value={role} options={[{ value: 'companion', label: 'Companion' }, { value: 'keeper', label: 'Keeper' }, { value: 'viewer', label: 'Viewer' }]} />
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
      <Segmented bind:value={newRole} options={[{ value: 'companion', label: 'Companion' }, { value: 'keeper', label: 'Keeper' }, { value: 'viewer', label: 'Viewer' }]} />
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
  .page { padding: 18px; max-width: 1080px; overflow: auto; height: 100%; }
  @media (max-width: 720px) { .page { padding: 12px 14px; } }
  .head { margin-bottom: 14px; }
  .section { font-size: 12px; font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .05em; margin: 20px 0 8px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--ink-3); display: inline-block; flex: none; }
  .dot.on { background: var(--ok); }
  .dot.relay { background: var(--warn); }
  .input { width: 100%; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px 12px; color: var(--ink); font: inherit; resize: none; }
  .ticket { width: 100%; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); padding: 8px; resize: none; }
  .code { display: flex; gap: 8px; justify-content: center; font: 700 30px var(--mono); margin: 12px 0 18px; }
  .code span { width: 40px; height: 52px; background: var(--surface-2); border-radius: 10px; display: grid; place-items: center; }
  .err { color: var(--risk); }
  .grow { flex: 1; min-width: 0; }
  .left { text-align: left; min-width: 0; }
  .ink { color: var(--ink); }
  .tiny { font-size: 11.5px; }
  .mono { font-family: var(--mono); }
  .card { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); }
  .activity { padding: 14px 16px; display: grid; gap: 9px; margin-bottom: 14px; }
  .headline { display: flex; align-items: center; gap: 10px; }
  .spinner { width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--info-soft); border-top-color: var(--info); animation: spin 1s linear infinite; flex: none; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
  .activity.idle .spinner { animation: none; border-color: var(--ok); background: var(--ok); }
  .activity.paused .spinner { animation: none; border-color: var(--warn); }
  .prog { height: 4px; border-radius: 2px; background: var(--surface-3); overflow: hidden; }
  .prog > span { display: block; height: 100%; background: var(--info); border-radius: 2px; transition: width .6s linear; }
  .prog.up > span { background: var(--ok); }
  .queue { display: grid; gap: 8px; border-top: 1px solid var(--line); padding-top: 10px; }
  .counts { display: flex; gap: 14px; flex-wrap: wrap; }
  .counts b { color: var(--ink); }
  .qrow { display: flex; gap: 10px; align-items: center; }
  .qthumb { width: 34px; height: 34px; border-radius: 7px; object-fit: cover; background: var(--surface-2); flex: none; }
  .act.up { color: var(--ok); }
  .act.down { color: var(--info); }
  .act.warn { color: var(--warn); }
  .act.ok { color: var(--ok); }
  .act.idle { color: var(--ink-3); }
  .table { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); }
  .thead, .trow { display: grid; grid-template-columns: minmax(150px, 1.2fr) minmax(110px, .8fr) minmax(200px, 1.6fr) minmax(160px, 1.3fr) 34px; gap: 14px; align-items: center; padding: 12px 14px; }
  .thead { color: var(--ink-3); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; padding-top: 10px; padding-bottom: 8px; border-bottom: 1px solid var(--line); }
  .trow + .trow { border-top: 1px solid var(--line); }
  .who { gap: 10px; min-width: 0; }
  .t1 { font-weight: 600; } .t2 { font-size: 12.5px; }
  .conn { gap: 8px; }
  .keeps b, .prow b { white-space: nowrap; }
  .keeps { display: grid; gap: 5px; }
  .menu-wrap { position: relative; }
  .kebab { border: 0; background: none; color: var(--ink-2); font-size: 18px; width: 32px; height: 32px; border-radius: 8px; cursor: pointer; }
  .kebab:hover { background: var(--surface-3); }
  .menu { position: absolute; right: 0; top: 36px; z-index: 5; background: var(--surface); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow); padding: 5px; min-width: 170px; display: grid; }
  .menu button { text-align: left; border: 0; background: none; color: var(--ink); font: inherit; font-size: 13.5px; padding: 8px 10px; border-radius: 7px; cursor: pointer; }
  .menu button:hover { background: var(--surface-2); }
  .menu .danger { color: var(--risk); }
  .prow { display: grid; gap: 8px; width: 100%; padding: 13px 14px; border: 0; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .prow + .prow { border-top: 1px solid var(--line); }
  .prow:active { background: var(--surface-2); }
  .prow .row-flex { gap: 10px; }
  .chev { color: var(--ink-3); font-size: 20px; }
  .kv { display: grid; grid-template-columns: auto 1fr; gap: 6px 14px; margin: 0; }
  .kv dt { color: var(--ink-2); } .kv dd { margin: 0; text-align: right; }
  .acts { display: grid; gap: 4px; border-top: 1px solid var(--line); padding-top: 8px; }
  .between { justify-content: space-between; }
  .bar { display: flex; height: 6px; border-radius: 3px; background: var(--surface-2); overflow: hidden; }
  .bar .orig { background: var(--accent); }
  .bar .prev { background: var(--ink-3); opacity: .5; }
</style>
