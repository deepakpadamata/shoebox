<script lang="ts">
  // Share an album (U1-21): three choices, Invite people, Web link, Send copies, with the members
  // below. Inviting hands the invitation text to the system share sheet; the QR code and the long
  // code are there for a person standing next to you.
  import { api, type InviteOut, type SharedMemberInfo } from '../lib/api';
  import { dateTime, plural } from '../lib/format';
  import { fail, refreshAlbums, toast } from '../lib/store.svelte';
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import QR from './QR.svelte';
  import Segmented from './Segmented.svelte';
  import Sheet from './Sheet.svelte';

  interface Props {
    open: boolean;
    name: string;
    /** The person's own album, if it is one: it can be shared, linked on the web and sent as copies. */
    personal?: string;
    /** The shared album, if it is shared already. */
    shared?: string;
    /** May invite people (the owner). */
    owner: boolean;
    onclose: () => void;
    onweblink: () => void;
  }
  let { open, name, personal, shared, owner, onclose, onweblink }: Props = $props();

  let view = $state<'choices' | 'location' | 'invite'>('choices');
  let role = $state('contributor');
  let invite = $state<InviteOut | null>(null);
  let showQr = $state(false);
  let members = $state<SharedMemberInfo[]>([]);
  let locationAsk = $state(0);
  let busy = $state(false);
  // The shared album behind this one, once the invitation made it shared.
  let sharedId = $state<string | undefined>(undefined);
  const sid = $derived(shared ?? sharedId);

  $effect(() => {
    if (!open) return;
    view = 'choices';
    invite = null;
    showQr = false;
    sharedId = undefined;
    if (shared) api.sharedMembers(shared).then((m) => (members = m)).catch(() => {});
    else members = [];
  });

  async function startInvite() {
    if (sid) { view = 'invite'; return; }
    // A personal album becomes a shared one the first time it is shared.
    busy = true;
    try {
      const check = await api.albumShareCheck(personal!);
      if (check.with_location > 0) { locationAsk = check.with_location; view = 'location'; }
      else await makeShared('remove');
    } catch (e) { fail(e); } finally { busy = false; }
  }
  async function makeShared(location: string) {
    busy = true;
    try {
      const s = await api.albumShare(personal!, location);
      sharedId = s.id;
      await refreshAlbums();
      members = await api.sharedMembers(s.id).catch(() => []);
      view = 'invite';
    } catch (e) { fail(e); } finally { busy = false; }
  }
  async function makeInvite(): Promise<InviteOut | null> {
    try {
      invite = await api.sharedInvite(sid!, role, 7);
      return invite;
    } catch (e) { fail(e); return null; }
  }
  async function send() {
    const inv = invite ?? (await makeInvite());
    if (!inv) return;
    api.shareText(`Join my Shoebox album “${name}”. Open Shoebox, choose Albums, then the plus, then Join with an invitation, and paste this:\n\n${inv.text}`).catch((e) => toast(e instanceof Error ? e.message : String(e)));
  }
  async function copy() {
    const inv = invite ?? (await makeInvite());
    if (!inv) return;
    try { await navigator.clipboard.writeText(inv.text); toast('Invitation copied'); } catch { toast('Select the text and copy it'); }
  }
  async function qr() {
    if (!invite) await makeInvite();
    showQr = true;
  }
  async function sendCopies() {
    busy = true;
    try {
      const ids: string[] = [];
      let cursor = null as Awaited<ReturnType<typeof api.albumPage>>['next'];
      do {
        const p = await api.albumPage(personal!, cursor, 500);
        ids.push(...p.items.map((i) => i.id));
        cursor = p.next;
      } while (cursor);
      if (!ids.length) { toast('This album has no photos yet'); return; }
      await api.shareFiles(ids);
      onclose();
    } catch (e) { fail(e); } finally { busy = false; }
  }
  // Changing who may do what restarts the invitation: the old text carried the old role.
  const pickRole = (r: string) => { role = r; invite = null; showQr = false; };
</script>

<Sheet {open} title={view === 'location' ? 'Share the location?' : `Share ${name}`} {onclose} width={460}>
  {#if view === 'location'}
    <div class="stack">
      <p>{plural(locationAsk, 'photo')} in this album {locationAsk === 1 ? 'was' : 'were'} taken at a known place. People you share it with would see where. Your choice also applies to photos you add later.</p>
      <Button kind="primary" block disabled={busy} onclick={() => makeShared('remove')}>Remove the location</Button>
      <Button block disabled={busy} onclick={() => makeShared('keep')}>Keep the location</Button>
    </div>
  {:else if view === 'invite'}
    <div class="stack">
      <div>
        <div class="label">They can</div>
        <Segmented value={role} onchange={pickRole} options={[{ value: 'contributor', label: 'View & add' }, { value: 'viewer', label: 'View only' }]} />
      </div>
      <Button kind="primary" block onclick={send}>Send invitation…</Button>
      <p class="muted small">It works once and for 7 days. You both need to be online when they join.</p>
      <div class="two">
        <Button onclick={qr}>Show QR code</Button>
        <Button onclick={copy}>Copy code</Button>
      </div>
      {#if showQr && invite}
        <div class="qrbox"><QR svg={invite.qr_svg} size={260} /></div>
        <p class="muted small">Valid until {dateTime(invite.expires_at)}. Anyone who has it can join, so send it only to the person you mean.</p>
      {/if}
      <Button kind="ghost" block onclick={() => (view = 'choices')}>Back</Button>
    </div>
  {:else}
    <div class="stack">
      <div class="choices">
        {#if (shared && owner) || personal}
          <button class="choice" disabled={busy} onclick={startInvite}><Icon name="people" size={22} /><span><b>Invite people</b><small>They join the album and see new photos as you add them.</small></span></button>
        {/if}
        {#if personal}
          <button class="choice" onclick={() => { onclose(); onweblink(); }}><Icon name="link" size={22} /><span><b>Web link</b><small>Anyone with the link can look, with nothing installed.</small></span></button>
          <button class="choice" disabled={busy} onclick={sendCopies}><Icon name="share" size={22} /><span><b>{busy ? 'Getting photos ready…' : 'Send copies'}</b><small>Send the photos to another app, as files.</small></span></button>
        {/if}
      </div>
      {#if members.length}
        <div class="label">People</div>
        <div class="members">
          {#each members as m (m.key)}
            <div class="member"><Icon name="person" size={18} /><span class="grow">{m.name}{m.me ? ' (you)' : ''}</span><span class="muted small">{m.role}</span></div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</Sheet>

<style>
  .label { font-size: var(--t-sm); font-weight: 600; color: var(--ink-3); text-transform: uppercase; letter-spacing: .04em; margin-bottom: 6px; }
  .choices { display: grid; border: 1px solid var(--line); border-radius: 12px; overflow: hidden; }
  .choice { display: flex; align-items: center; gap: 12px; padding: 12px 14px; min-height: var(--tap); border: 0; border-top: 1px solid var(--line); background: var(--surface); color: var(--ink); font: inherit; text-align: left; cursor: pointer; }
  .choice:first-child { border-top: 0; }
  .choice:hover:not(:disabled) { background: var(--surface-2); }
  .choice:disabled { opacity: .6; }
  .choice span { display: grid; }
  .choice small { color: var(--ink-2); font-size: var(--t-sm); }
  .two { display: flex; gap: 8px; }
  .two :global(button) { flex: 1; }
  .qrbox { display: grid; place-items: center; }
  .members { display: grid; }
  .member { display: flex; align-items: center; gap: 10px; padding: 8px 2px; border-top: 1px solid var(--line); }
  .member:first-child { border-top: 0; }
  .grow { flex: 1; }
</style>
