<script lang="ts">
  // Add photos to an album from the album's own page (U1-19): choose from the library. A shared
  // album asks about location first when the photos have one and the album says to ask.
  import { api, blobUrl, type PhotoSummary } from '../lib/api';
  import { plural } from '../lib/format';
  import { fail, toast } from '../lib/store.svelte';
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import Sheet from './Sheet.svelte';
  import Toggle from './Toggle.svelte';

  interface Props {
    open: boolean;
    name: string;
    /** The personal album, when it is one: photos go straight in (and are shared if it is). */
    personal?: string;
    /** The shared album, when it is one. */
    shared?: string;
    onclose: () => void;
    ondone: () => void;
  }
  let { open, name, personal, shared, onclose, ondone }: Props = $props();

  let library = $state<PhotoSummary[]>([]);
  let picked = $state<string[]>([]);
  let askOpen = $state(false);
  let withLocation = $state(0);
  let remember = $state(false);
  let adding = $state(false);

  $effect(() => {
    if (!open) return;
    picked = [];
    askOpen = false;
    api.libraryPage(null, 600).then((p) => (library = p.items)).catch(() => (library = []));
  });

  async function addPicked() {
    try {
      if (shared) {
        const check = await api.sharedAddCheck(shared, picked);
        if (check.with_location > 0 && check.policy === 'ask') {
          withLocation = check.with_location;
          askOpen = true;
          return;
        }
      }
      await doAdd(null);
    } catch (e) { fail(e); }
  }
  async function doAdd(location: string | null) {
    adding = true;
    try {
      if (shared) {
        const r = await api.sharedAdd(shared, picked, location, remember);
        toast(`Added ${plural(r.added, 'photo')}${r.location_removed ? ` (location removed from ${r.location_removed})` : ''}${r.skipped.length ? `. Left out ${r.skipped.length}: ${r.skipped[0]}` : ''}`);
      } else if (personal) {
        await api.albumAdd(personal, picked);
        toast(`Added ${plural(picked.length, 'photo')}`);
      }
      askOpen = false;
      picked = [];
      ondone();
    } catch (e) { fail(e); } finally { adding = false; }
  }
</script>

<Sheet {open} title="Add photos to {name}" {onclose} width={680}>
  <div class="stack">
    <p class="muted small">Choose from your library. {picked.length ? `${plural(picked.length, 'photo')} chosen.` : ''}</p>
    <div class="pick">
      {#each library as p (p.id)}
        <button class="ptile" class:sel={picked.includes(p.id)} onclick={() => (picked = picked.includes(p.id) ? picked.filter((x) => x !== p.id) : [...picked, p.id])} aria-label="Photo" aria-pressed={picked.includes(p.id)}>
          {#if p.has_thumb && !p.is_video}<img src={blobUrl(p.id, 'thumb')} alt="" loading="lazy" />{/if}
          <span class="check">{#if picked.includes(p.id)}<Icon name="check" size={14} />{/if}</span>
        </button>
      {/each}
    </div>
    <Button kind="primary" block disabled={!picked.length || adding} onclick={addPicked}>{adding ? 'Adding…' : `Add ${plural(picked.length, 'photo')}`}</Button>
  </div>
</Sheet>

<Sheet open={askOpen} title="Share the location?" onclose={() => (askOpen = false)} width={420}>
  <div class="stack">
    <p>{plural(withLocation, 'photo')} {withLocation === 1 ? 'was' : 'were'} taken at a known place. Everyone in this album would see where.</p>
    <Button kind="primary" block disabled={adding} onclick={() => doAdd('remove')}>Remove the location</Button>
    <Button block disabled={adding} onclick={() => doAdd('keep')}>Keep the location</Button>
    <label class="row-flex small"><Toggle checked={remember} label="Remember for this album" onchange={(v) => (remember = v)} /> Remember for this album</label>
    <p class="muted small">Removing the location changes only the shared copy; your own photo keeps it. Some formats (such as HEIC) cannot be cleaned yet and are left out.</p>
  </div>
</Sheet>

<style>
  .pick { display: grid; grid-template-columns: repeat(auto-fill, minmax(84px, 1fr)); gap: 3px; max-height: 50vh; overflow: auto; }
  .ptile { position: relative; aspect-ratio: 1; border: 0; padding: 0; background: var(--surface-2); cursor: pointer; overflow: hidden; }
  .ptile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .check { position: absolute; right: 5px; top: 5px; width: 18px; height: 18px; border-radius: 50%; border: 2px solid #fff; background: rgba(0,0,0,.15); color: #fff; display: grid; place-items: center; }
  .sel .check { background: var(--accent); }
  .sel::after { content: ''; position: absolute; inset: 0; background: rgba(255,255,255,.25); pointer-events: none; }
</style>
