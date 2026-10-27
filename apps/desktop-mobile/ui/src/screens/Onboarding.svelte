<script lang="ts">
  // First run (mockups 1a–1d): create a library, join one with a pairing ticket, or recover.
  import { api, type CreatedLibrary } from '../lib/api';
  import { app, fail, refreshState } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';
  import Segmented from '../components/Segmented.svelte';
  import { canScan, scanQr } from '../lib/scan';
  import RecoveryKit from './RecoveryKit.svelte';

  type Step = 'welcome' | 'create' | 'kit' | 'join' | 'code' | 'recover';
  let step = $state<Step>('welcome');
  let name = $state('');
  // A phone or tablet is a Companion; a computer is usually the keeper.
  const handheld = /Android|iPhone|iPad/i.test(navigator.userAgent);
  const thisDevice = handheld ? 'this phone' : 'this computer';
  const defaultName = handheld ? 'My phone' : 'This computer';
  let role = $state(handheld ? 'companion' : 'keeper');
  let ticket = $state('');
  let link = $state('');
  let phrase = $state('');
  let busy = $state(false);
  let created = $state<CreatedLibrary | null>(null);

  async function create() {
    busy = true;
    try {
      created = await api.createLibrary(name.trim() || defaultName, role);
      step = 'kit';
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  async function join() {
    busy = true;
    app.pairingError = '';
    step = 'code';
    try {
      await api.join(ticket.trim(), name.trim() || defaultName);
      await refreshState();
    } catch (e) {
      app.pairingError = e instanceof Error ? e.message : String(e);
      step = 'join';
    } finally {
      busy = false;
    }
  }

  async function scan() {
    try {
      const text = await scanQr();
      if (text) ticket = text.trim();
    } catch (e) {
      app.pairingError = e instanceof Error ? e.message : String(e);
    }
  }

  async function recover() {
    busy = true;
    try {
      await api.recover(link.trim(), phrase, name.trim() || defaultName);
      await refreshState();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  async function answer(accept: boolean) {
    await api.confirmPairing(accept).catch(fail);
    if (!accept) {
      app.pairing = null;
      step = 'join';
    }
  }
</script>

<div class="onboarding">
  <div class="card panel">
    {#if step === 'welcome'}
      <div class="logo">📦</div>
      <h1>Shoebox</h1>
      <p class="muted">Your photos, on your own devices. No account, no cloud you do not control.</p>
      <div class="stack">
        <Button kind="primary" block onclick={() => (step = 'create')}>Create a new library</Button>
        <Button block onclick={() => (step = 'join')}>Add {thisDevice} to an existing library</Button>
        <Button kind="ghost" block onclick={() => (step = 'recover')}>I lost my devices: restore with my recovery phrase</Button>
      </div>
    {:else if step === 'create'}
      <h2>Name this device</h2>
      <p class="muted small">Other devices will show this name.</p>
      <div class="stack">
        <input class="input" placeholder={handheld ? 'Pixel 9' : 'MacBook Pro'} bind:value={name} />
        <div>
          <div class="label">What does this device do?</div>
          <Segmented bind:value={role} options={[{ value: 'keeper', label: 'Keeper' }, { value: 'companion', label: 'Companion' }, { value: 'viewer', label: 'Viewer' }]} />
          <p class="small muted" style="margin-top:6px">
            {role === 'keeper' ? 'Keeps a full copy of every original. Best for a computer or a home server that is usually on.' : role === 'companion' ? 'Keeps recent and favourite photos in full and previews of the rest; older originals live on your keepers and are fetched when you open them.' : 'Browses only; never stores originals.'}
          </p>
        </div>
        <Button kind="primary" block disabled={busy} onclick={create}>{busy ? 'Creating…' : 'Create library'}</Button>
        <Button kind="ghost" block onclick={() => (step = 'welcome')}>Back</Button>
      </div>
    {:else if step === 'kit' && created}
      <h2>Save your recovery kit</h2>
      <RecoveryKit phrase={created.phrase} kitText={created.kit_text} onsaved={() => { created = null; }} />
    {:else if step === 'join'}
      <h2>Add {thisDevice}</h2>
      <p class="muted small">On a device that is already in the library, open <b>Devices → Add a device</b> and paste the code it shows.</p>
      <div class="stack">
        <input class="input" placeholder="Name this device" bind:value={name} />
        {#if canScan()}<Button block onclick={scan}>Scan the QR code</Button>{/if}
        <textarea class="input mono" rows="3" placeholder="shoebox-pair1…" bind:value={ticket}></textarea>
        {#if app.pairingError}<div class="err small">{app.pairingError}</div>{/if}
        <Button kind="primary" block disabled={busy || !ticket.trim()} onclick={join}>Connect</Button>
        <Button kind="ghost" block onclick={() => (step = 'welcome')}>Back</Button>
      </div>
    {:else if step === 'code'}
      <h2>Check the code</h2>
      {#if app.pairing}
        <p class="muted small">Does <b>{app.pairing.peer_name}</b> show the same six digits? Only continue if they match.</p>
        <div class="code" aria-label="Pairing code">{#each app.pairing.sas.split('') as d}<span>{d}</span>{/each}</div>
        <div class="stack">
          <Button kind="primary" block onclick={() => answer(true)}>They match</Button>
          <Button block onclick={() => answer(false)}>They do not match</Button>
        </div>
      {:else}
        <p class="muted">Connecting to the other device…</p>
      {/if}
    {:else if step === 'recover'}
      <h2>Restore with your recovery phrase</h2>
      <p class="muted small">You need the 24 words and the link of any device that still has the library (<span class="mono">shoebox-keeper address</span> on a keeper).</p>
      <div class="stack">
        <input class="input" placeholder="Name this device" bind:value={name} />
        <input class="input mono" placeholder="shoebox-peer1…" bind:value={link} />
        <textarea class="input" rows="3" placeholder="24-word recovery phrase" bind:value={phrase}></textarea>
        <Button kind="primary" block disabled={busy || !link.trim() || !phrase.trim()} onclick={recover}>{busy ? 'Waiting for the other device…' : 'Restore'}</Button>
        {#if busy}<p class="muted small">If that device has a screen, allow this there within two minutes. A keeper lets it through by itself.</p>{/if}
        <Button kind="ghost" block onclick={() => (step = 'welcome')}>Back</Button>
      </div>
    {/if}
  </div>
</div>

<style>
  .onboarding { height: 100%; display: grid; place-items: center; background: var(--bg); padding: 20px; overflow: auto; }
  .panel { width: min(460px, 100%); padding: 28px 28px 24px; }
  .logo { font-size: 44px; text-align: center; }
  h1, h2 { margin: 6px 0 8px; letter-spacing: -0.02em; }
  h1 { text-align: center; font-size: 28px; }
  h2 { font-size: 21px; }
  p.muted { margin: 0 0 16px; }
  .label { font-size: 12px; font-weight: 700; color: var(--ink-2); margin-bottom: 6px; text-transform: uppercase; letter-spacing: .04em; }
  .input { width: 100%; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 10px 12px; color: var(--ink); font: inherit; }
  .code { display: flex; gap: 8px; justify-content: center; font: 700 30px var(--mono); margin: 12px 0 18px; }
  .code span { width: 40px; height: 52px; background: var(--surface-2); border-radius: 10px; display: grid; place-items: center; }
  .err { color: var(--risk); }
</style>
