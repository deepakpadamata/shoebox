<script lang="ts">
  // Share an album as a web link (W1-07): anyone with the link can look at it in a browser, with nothing installed.
  // The photos go up encrypted; the key is in the link and nowhere else.
  import { api, type WebLinkCheck, type WebLinkInfo, type WebLinksInfo } from '../lib/api';
  import { bytes, dateTime, plural } from '../lib/format';
  import { fail, toast } from '../lib/store.svelte';
  import Sheet from './Sheet.svelte';
  import Button from './Button.svelte';
  import Toggle from './Toggle.svelte';
  import QR from './QR.svelte';

  interface Props { open: boolean; album: string; onclose: () => void }
  let { open, album, onclose }: Props = $props();

  let info = $state<WebLinksInfo | null>(null);
  let check = $state<WebLinkCheck | null>(null);
  let fullSize = $state(false);
  let live = $state(false);
  let expiry = $state('never');
  let busy = $state('');
  let url = $state('');
  let phrase = $state('');
  let shown = $state<string | null>(null);

  const DAYS: Record<string, number | null> = { never: null, '7': 7, '30': 30, '90': 90 };

  async function load() {
    try {
      info = await api.webLinks(album);
      if (info.url) url = info.url;
    } catch (e) { fail(e); }
  }
  async function loadCheck() {
    try { check = await api.webLinkCheck(album, fullSize); } catch (e) { fail(e); }
  }

  $effect(() => {
    if (!open) return;
    shown = null; busy = ''; phrase = ''; fullSize = false; live = false; expiry = 'never'; check = null;
    void load();
    void loadCheck();
  });

  async function setup() {
    busy = 'setup';
    try {
      await api.cloudSetup(url.trim() || null, phrase.trim() || null);
      phrase = '';
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  async function make() {
    busy = 'make';
    try {
      const link = await api.webLinkCreate(album, fullSize, DAYS[expiry] ?? null, live);
      shown = link.share_id;
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  async function copy(l: WebLinkInfo) {
    try { await navigator.clipboard.writeText(l.text); toast('The link was copied'); } catch { toast('Select the link and copy it'); }
  }

  async function revoke(l: WebLinkInfo) {
    busy = l.share_id;
    try {
      await api.webLinkRevoke(album, l.share_id);
      if (shown === l.share_id) shown = null;
      toast('The link no longer works');
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  async function rotate(l: WebLinkInfo) {
    busy = l.share_id;
    try {
      const n = await api.webLinkRotate(album, l.share_id);
      shown = n.share_id;
      toast('The old link stopped; this is the new one');
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  async function refresh() {
    busy = 'refresh';
    try {
      const n = await api.webLinksRefresh();
      toast(n ? `Updated ${plural(n, 'link')}` : 'Everything is up to date');
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  const ready = $derived(!!info && info.enabled && !!info.url);
</script>

<Sheet {open} title="Share a link" {onclose} width={440}>
  {#if !info}
    <p class="muted small">Opening…</p>
  {:else if !ready}
    <div class="stack">
      <p>Anyone you send the link to sees this album in their browser, with nothing installed. The photos are encrypted before they leave this device, so the service that stores them cannot see them.</p>
      {#if !info.url}
        <label class="field">Service address
          <input class="input" placeholder="https://" bind:value={url} disabled={busy !== ''} />
        </label>
      {/if}
      {#if !info.enabled}
        <label class="field">Recovery phrase, once
          <textarea class="input" rows="3" placeholder="The 24 words from your recovery kit" bind:value={phrase} disabled={busy !== ''}></textarea>
          <span class="muted small">It gives this library the key that encrypts what it stores online. It is checked here and not kept or sent anywhere.</span>
        </label>
      {/if}
      <Button kind="primary" block disabled={busy !== '' || (!info.url && !url.trim()) || (!info.enabled && !phrase.trim())} onclick={setup}>{busy === 'setup' ? 'Setting up…' : 'Set up web links'}</Button>
    </div>
  {:else}
    <div class="stack">
      {#if info.links.length}
        <div class="label">Links to this album</div>
        {#each info.links as l (l.share_id)}
          <div class="link" class:fresh={shown === l.share_id}>
            <div class="row-flex">
              <span class="grow small"><b>{plural(l.items, 'item')}</b>{l.full_size ? ', full size' : ''}{l.expires_at ? `, until ${dateTime(l.expires_at)}` : ''}</span>
              {#if l.live}<span class="pill small">{l.online ? 'From this device' : 'From another device'}</span>{/if}
              {#if l.stale}<span class="pill small">Album changed</span>{/if}
            </div>
            {#if l.skipped || l.withheld}
              <p class="small muted">{#if l.skipped}{plural(l.skipped, 'photo')} not on this device yet, left out. {/if}{#if l.withheld}{plural(l.withheld, 'original')} left out because their location cannot be removed.{/if}</p>
            {/if}
            {#if l.live}
              <p class="small muted">Works only while {l.online ? 'this device or another one of yours that holds the album' : 'a device of yours that holds the album'} is on and online. Anyone opening it while they are all off sees that it is not available right now.</p>
            {/if}
            {#if shown === l.share_id}
              <div class="qr"><QR svg={l.qr_svg} size={170} /></div>
            {/if}
            <input class="input mono" readonly value={l.text} aria-label="Link" onfocus={(e) => e.currentTarget.select()} />
            <div class="row-flex wrap">
              <Button size="sm" kind="primary" onclick={() => copy(l)}>Copy</Button>
              <Button size="sm" onclick={() => (shown = shown === l.share_id ? null : l.share_id)}>{shown === l.share_id ? 'Hide QR' : 'QR code'}</Button>
              <Button size="sm" disabled={busy !== ''} onclick={() => rotate(l)}>New link</Button>
              <Button size="sm" kind="danger" disabled={busy !== ''} onclick={() => revoke(l)}>Stop sharing</Button>
            </div>
          </div>
        {/each}
        {#if info.links.some((l) => l.stale)}
          <Button block disabled={busy !== ''} onclick={refresh}>{busy === 'refresh' ? 'Updating…' : 'Update links with the album'}</Button>
        {/if}
        <div class="label">Another link</div>
      {:else}
        <p>Anyone with the link sees this album in a browser. It works even when your devices are off.</p>
      {/if}

      <div class="choice" role="radiogroup" aria-label="Where the photos are kept">
        <label class:on={!live}><input type="radio" name="where" checked={!live} onchange={() => (live = false)} /><b>Kept online</b><span class="small muted">Works even when all your devices are off. Uses storage with the service.</span></label>
        <label class:on={live}><input type="radio" name="where" checked={live} onchange={() => (live = true)} /><b>From my devices</b><span class="small muted">Free and nothing is stored online, but it works only while a device of yours is on.</span></label>
      </div>
      <div class="row-flex"><span class="grow">Allow full-size downloads</span><Toggle checked={fullSize} label="Allow full-size downloads" onchange={(v) => { fullSize = v; void loadCheck(); }} /></div>
      <label class="row-flex">Link works for
        <select class="input grow" bind:value={expiry} disabled={busy !== ''}>
          <option value="never">As long as I keep it</option>
          <option value="7">7 days</option>
          <option value="30">30 days</option>
          <option value="90">90 days</option>
        </select>
      </label>

      {#if check}
        <p class="small muted">
          {#if live}
            This keeps an encrypted copy of up to <b>{bytes(check.bytes)}</b> for {plural(check.photos, 'photo')}{check.videos ? ` and ${plural(check.videos, 'video')}` : ''} on this device and serves it to people with the link while the device is online. Nothing is uploaded.
          {:else}
            This uploads up to <b>{bytes(check.bytes)}</b> for {plural(check.photos, 'photo')}{check.videos ? ` and ${plural(check.videos, 'video')}` : ''}, encrypted, to {info.url}. Only people with the link can see them, and nothing is sent until you tap the button.
          {/if}
          {#if fullSize && check.with_location}
            The location is removed from {plural(check.with_location - check.unremovable, 'full-size photo')}{check.unremovable ? `; ${plural(check.unremovable, 'photo')} in a format that cannot be cleaned (such as HEIC) will be left out at full size` : ''}.
          {/if}
        </p>
      {/if}
      <Button kind="primary" block disabled={busy !== '' || (check !== null && check.photos + check.videos === 0)} onclick={make}>{busy === 'make' ? (live ? 'Preparing…' : 'Uploading…') : info.links.length ? 'Make another link' : 'Make a link'}</Button>
      <p class="small muted">A link can be copied by anyone who has it. Stopping it makes it fail at once, but cannot take back what someone already downloaded.</p>
    </div>
  {/if}
</Sheet>

<style>
  .label { font-size: 12px; font-weight: 600; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .link { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid var(--line); border-radius: 12px; }
  .link.fresh { border-color: var(--accent); }
  .qr { display: flex; justify-content: center; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
  textarea.input { height: auto; padding: 10px 12px; resize: vertical; }
  .mono { font-family: ui-monospace, Menlo, monospace; font-size: 12px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-weight: 600; font-size: 13px; }
  .field .muted { font-weight: 400; }
  .wrap { flex-wrap: wrap; }
  .choice { display: flex; flex-direction: column; gap: 8px; }
  .choice label { display: grid; grid-template-columns: auto 1fr; column-gap: 8px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 12px; cursor: pointer; }
  .choice label.on { border-color: var(--accent); }
  .choice span { grid-column: 2; }
  .pill { background: var(--surface-3); border-radius: 999px; padding: 2px 8px; }
</style>
