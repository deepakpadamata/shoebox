<script lang="ts">
  // Share an album as a web link (W1-07, W1-08): one link for the album, and below it the options that change what
  // the link is. Anyone with the link sees the album in a browser with nothing installed; the photos are encrypted
  // before they leave this device and the key is in the link and nowhere else.
  import { untrack } from 'svelte';
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
  // The options as the person has set them; they start from the link and only change it when applied.
  let live = $state(false);
  let fullSize = $state(false);
  let expiry = $state('never');
  let busy = $state('');
  let url = $state('');
  let phrase = $state('');
  let qr = $state(false);

  const DAYS: Record<string, number | null> = { never: null, current: null, '7': 7, '30': 30, '90': 90 };

  // The newest link is the album's link; older ones (made before there was just one) are listed under it.
  const link = $derived<WebLinkInfo | null>(info && info.links.length ? [...info.links].sort((a, b) => b.created_at - a.created_at)[0] : null);
  const older = $derived<WebLinkInfo[]>(info && link ? info.links.filter((l) => l.share_id !== link.share_id) : []);
  const ready = $derived(!!info && info.enabled && !!info.url);
  const changed = $derived(!!link && (live !== link.live || fullSize !== link.full_size || (link.expires_at ? expiry !== 'current' : expiry !== 'never')));
  const newAddress = $derived(!!link && live !== link.live);

  function adopt(l: WebLinkInfo | null) {
    live = l?.live ?? false;
    fullSize = l?.full_size ?? false;
    expiry = l?.expires_at ? 'current' : 'never';
  }

  async function load(keepOptions = false) {
    try {
      info = await api.webLinks(album);
      if (info.url) url = info.url;
      if (!keepOptions) adopt(info.links.length ? [...info.links].sort((a, b) => b.created_at - a.created_at)[0] : null);
    } catch (e) { fail(e); }
  }
  async function loadCheck() {
    try { check = await api.webLinkCheck(album, fullSize); } catch (e) { fail(e); }
  }

  // Only opening the sheet (or another album) resets it; reading the options in here must not re-run it.
  $effect(() => {
    if (!open) return;
    album;
    untrack(() => {
      busy = ''; phrase = ''; check = null; qr = false;
      void load().then(loadCheck);
    });
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
      await api.webLinkCreate(album, fullSize, DAYS[expiry] ?? null, live);
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  // Same kind of link: the address stays and its contents change. A different kind needs a new address, and the
  // old one stops.
  async function apply() {
    if (!link) return;
    busy = 'apply';
    try {
      const days = expiry === 'current' ? undefined : (DAYS[expiry] ?? null);
      if (newAddress) {
        await api.webLinkCreate(album, fullSize, days ?? null, live);
        await api.webLinkRevoke(album, link.share_id);
        toast('This is a new link; the old one stopped');
      } else {
        // Keeping the current end date: send it back as the days that are left.
        const left = expiry === 'current' && link.expires_at ? Math.max(1, Math.ceil((link.expires_at - Date.now()) / 86_400_000)) : (days ?? null);
        await api.webLinkUpdate(album, link.share_id, fullSize, left);
        toast('The link was updated');
      }
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  async function copy() {
    if (!link) return;
    try { await navigator.clipboard.writeText(link.text); toast('The link was copied'); } catch { toast('Select the link and copy it'); }
  }

  async function stop(l: WebLinkInfo) {
    busy = l.share_id;
    try {
      await api.webLinkRevoke(album, l.share_id);
      toast('The link no longer works');
      await load();
    } catch (e) { fail(e); } finally { busy = ''; }
  }

  async function refresh() {
    busy = 'refresh';
    try {
      const n = await api.webLinksRefresh();
      toast(n ? 'The link was brought up to date' : 'Everything is up to date');
      await load(true);
    } catch (e) { fail(e); } finally { busy = ''; }
  }
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
      {#if link}
        <div class="link">
          <div class="row-flex">
            <span class="grow small"><b>{plural(link.items, 'item')}</b>{link.expires_at ? `, until ${dateTime(link.expires_at)}` : ''}</span>
            {#if link.stale}<span class="pill small">Album changed</span>{/if}
          </div>
          {#if link.reported !== null}
            <p class="small warn"><b>Switched off.</b> Someone who opened this link reported it{link.reported ? `: “${link.reported}”` : ''}. It stays off until the service has looked at it. You can stop sharing, or make a new link.</p>
          {/if}
          {#if link.skipped || link.withheld}
            <p class="small muted">{#if link.skipped}{plural(link.skipped, 'photo')} not on this device yet, left out. {/if}{#if link.withheld}{plural(link.withheld, 'original')} left out because their location cannot be removed.{/if}</p>
          {/if}
          {#if qr}<div class="qr"><QR svg={link.qr_svg} size={170} /></div>{/if}
          <input class="input mono" readonly value={link.text} aria-label="Link" onfocus={(e) => e.currentTarget.select()} />
          <div class="row-flex wrap">
            <Button size="sm" kind="primary" onclick={copy}>Copy</Button>
            <Button size="sm" onclick={() => (qr = !qr)}>{qr ? 'Hide QR' : 'QR code'}</Button>
            {#if link.stale}<Button size="sm" disabled={busy !== ''} onclick={refresh}>{busy === 'refresh' ? 'Updating…' : 'Update with the album'}</Button>{/if}
          </div>
        </div>
        <div class="label">How it is shared</div>
      {:else}
        <p>Anyone with the link sees this album in a browser, with nothing installed.</p>
      {/if}

      <div class="choice" role="radiogroup" aria-label="Where the photos are kept">
        <label class:on={!live}><input type="radio" name="where" checked={!live} disabled={busy !== ''} onchange={() => (live = false)} /><b>Kept online</b><span class="small muted">Works even when all your devices are off. Uses storage with the service.</span></label>
        <label class:on={live}><input type="radio" name="where" checked={live} disabled={busy !== ''} onchange={() => (live = true)} /><b>From my devices</b><span class="small muted">Free, and nothing is stored online, but it works only while a device of yours is on.</span></label>
      </div>
      <div class="row-flex"><span class="grow">Allow full-size downloads</span><Toggle checked={fullSize} label="Allow full-size downloads" onchange={(v) => { fullSize = v; void loadCheck(); }} /></div>
      <label class="row-flex">Link works for
        <select class="input grow" bind:value={expiry} disabled={busy !== ''}>
          {#if link?.expires_at}<option value="current">Until {dateTime(link.expires_at)}</option>{/if}
          <option value="never">As long as I keep it</option>
          <option value="7">7 days from now</option>
          <option value="30">30 days from now</option>
          <option value="90">90 days from now</option>
        </select>
      </label>

      {#if check && (!link || changed)}
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

      {#if !link}
        <Button kind="primary" block disabled={busy !== '' || (check !== null && check.photos + check.videos === 0)} onclick={make}>{busy === 'make' ? (live ? 'Preparing…' : 'Uploading…') : 'Make a link'}</Button>
      {:else if changed}
        {#if newAddress}<p class="small muted">Changing where the photos are kept makes a new link. The one above stops working, so anyone who has it needs the new one.</p>{/if}
        <Button kind="primary" block disabled={busy !== ''} onclick={apply}>{busy === 'apply' ? 'Applying…' : newAddress ? 'Make the new link' : 'Apply changes'}</Button>
      {/if}

      {#if link}
        <Button kind="danger" block disabled={busy !== ''} onclick={() => stop(link)}>Stop sharing</Button>
      {/if}
      {#if older.length}
        <p class="small muted">{plural(older.length, 'older link')} to this album still {older.length === 1 ? 'works' : 'work'}. <button class="linkish" type="button" disabled={busy !== ''} onclick={async () => { for (const l of older) await stop(l); }}>Stop {older.length === 1 ? 'it' : 'them'}</button></p>
      {/if}
      <p class="small muted">A link can be copied by anyone who has it. Stopping it makes it fail at once, but cannot take back what someone already downloaded.</p>
    </div>
  {/if}
</Sheet>

<style>
  .label { font-size: 12px; font-weight: 600; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .link { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid var(--line); border-radius: 12px; }
  .qr { display: flex; justify-content: center; }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; }
  textarea.input { height: auto; padding: 10px 12px; resize: vertical; }
  .mono { font-family: ui-monospace, Menlo, monospace; font-size: 12px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-weight: 600; font-size: 13px; }
  .field .muted { font-weight: 400; }
  .wrap { flex-wrap: wrap; }
  .linkish { background: none; border: 0; padding: 0; color: var(--accent); font: inherit; text-decoration: underline; cursor: pointer; }
  .warn { color: var(--danger, #c0392b); }
  .choice { display: flex; flex-direction: column; gap: 8px; }
  .choice label { display: grid; grid-template-columns: auto 1fr; column-gap: 8px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 12px; cursor: pointer; }
  .choice label.on { border-color: var(--accent); }
  .choice span { grid-column: 2; }
  .pill { background: var(--surface-3); border-radius: 999px; padding: 2px 8px; }
</style>
