<script lang="ts">
  // Import from Google Photos (I1-10): how to get the export, choose its files, see what will happen, run it in
  // the background (it can be stopped and continued; the app remembers it across a restart), and read the report.
  import { api, type ArchiveImportInfo, type ArchivePlanInfo } from '../lib/api';
  import { bytes, plural } from '../lib/format';
  import { canPick, pickArchives, pickFolder } from '../lib/pick';
  import { app, fail } from '../lib/store.svelte';
  import Sheet from './Sheet.svelte';
  import Button from './Button.svelte';
  import ProgressBar from './ProgressBar.svelte';

  interface Props { open: boolean; onclose: () => void; /** For the component gallery: show this state without the app core. */ preview?: { status: ArchiveImportInfo; plan?: ArchivePlanInfo } }
  let { open, onclose, preview }: Props = $props();

  let paths = $state<string[]>([]);
  let typed = $state('');
  let plan = $state<ArchivePlanInfo | null>(null);
  let busy = $state(false);
  let planError = $state('');

  const status = $derived(preview?.status ?? app.archive);
  const phase = $derived(status?.state ?? 'idle');
  const shownPlan = $derived(preview?.plan ?? plan);
  const progress = $derived(app.importProgress);
  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const short = $derived(!!shownPlan && shownPlan.free_bytes !== null && shownPlan.needed_bytes > shownPlan.free_bytes);

  async function choose(chosen: string[]) {
    if (chosen.length === 0) return;
    busy = true;
    planError = '';
    plan = null;
    try {
      paths = chosen;
      plan = await api.planArchiveImport(chosen);
    } catch (e) {
      paths = [];
      planError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function start(chosen: string[]) {
    busy = true;
    try {
      await api.startArchiveImport(chosen);
      plan = null;
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  async function stop() {
    try { await api.stopArchiveImport(); } catch (e) { fail(e); }
  }

  async function finish() {
    try { await api.forgetArchiveImport(); } catch (e) { fail(e); }
    paths = [];
    plan = null;
    onclose();
  }

  const reasons = (list: string[]) => list.slice(0, 3).map((l) => l.replace(/^.*?: /, ''));
</script>

<Sheet {open} title="Import from Google Photos" {onclose} width={480}>
  <div class="stack">
    {#if phase === 'running'}
      <p>Importing your Google Photos export. You can close this window: it carries on, and the toolbar shows how far it has got.</p>
      {#if progress}
        <ProgressBar total={progress.total} segments={[{ value: progress.done, tone: 'info' }]} label="Importing" />
        <p class="muted small">{progress.done.toLocaleString('en')} of {progress.total.toLocaleString('en')}</p>
      {:else}
        <p class="muted small">Reading the files…</p>
      {/if}
      <Button block onclick={stop}>Stop</Button>
    {:else if phase === 'done' && status?.report}
      {@const r = status.report}
      <p><b>{plural(r.imported, 'photo')}</b> came over{r.duplicates ? `; ${r.duplicates.toLocaleString('en')} already in your library were skipped` : ''}.</p>
      <ul class="small facts">
        {#if r.albums.length}<li>{plural(r.albums.length, 'album')}: {r.albums.map((a) => `${a.title} (${a.photos})`).join(', ')}</li>{/if}
        {#if r.export_dates}<li>Date taken from Google Photos for {plural(r.export_dates, 'photo')} (the file had none, or you had changed it there).</li>{/if}
        {#if r.export_places}<li>Place taken from Google Photos for {plural(r.export_places, 'photo')}.</li>{/if}
        {#if r.favorites}<li>{plural(r.favorites, 'favourite')} kept.</li>{/if}
        {#if r.trashed}<li>Left out, because they were in Google Photos' bin: {plural(r.trashed, 'photo')}.</li>{/if}
        {#if r.skipped}<li>{plural(r.skipped, 'file')} could not be read as photos or videos.</li>{/if}
        {#if r.unmatched}<li>No matching Google data for {plural(r.unmatched, 'photo')}: they came over with what the file itself says.</li>{/if}
      </ul>
      {#if r.part_errors.length || r.failed.length}
        <p class="small warn">{plural(r.part_errors.length + r.failed.length, 'problem')}: {[...reasons(r.part_errors), ...reasons(r.failed)].slice(0, 3).join('; ')}. Choose the same files again to try the rest.</p>
      {/if}
      <Button kind="primary" block onclick={finish}>Done</Button>
    {:else if phase === 'stopped'}
      <p>An import from Google Photos stopped before it finished ({plural(status?.paths.length ?? 0, 'file')}). Everything it had done is kept; continuing picks up where it stopped.</p>
      {#if status?.error}<p class="small warn">{status.error}</p>{/if}
      <Button kind="primary" block disabled={busy} onclick={() => start(status!.paths)}>Continue</Button>
      <Button block disabled={busy} onclick={finish}>Forget it</Button>
    {:else if shownPlan}
      {@const p = shownPlan}
      {#if !p.is_takeout}
        <p class="small warn">These files do not look like a Google Photos export (no album folders or JSON files from Google). Their photos and videos will import as plain files.</p>
      {/if}
      <p><b>{plural(p.photos, 'photo and video', 'photos and videos')}</b>{p.already ? ` (${p.already.toLocaleString('en')} already done by an earlier run)` : ''}{p.albums.length ? `, in ${plural(p.albums.length, 'album')}` : ''}.</p>
      <ul class="small facts">
        {#if p.copies}<li>{plural(p.copies, 'copy', 'copies')} in album and year folders are imported once.</li>{/if}
        {#if p.trashed}<li>Left out, because they are in Google Photos' bin: {plural(p.trashed, 'photo')}.</li>{/if}
        {#if p.with_export_data}<li>Dates, places or favourites from Google for {plural(p.with_export_data, 'photo')}.</li>{/if}
        {#if p.unmatched + p.ambiguous}<li>No matching Google data for {plural(p.unmatched + p.ambiguous, 'photo')}: they import as plain files.</li>{/if}
        {#if p.not_photos}<li>Skipped, not photos or videos: {plural(p.not_photos, 'other file')}.</li>{/if}
      </ul>
      <p class="small" class:warn={short}>Needs about {bytes(p.needed_bytes)} of free space{p.free_bytes !== null ? `; this Mac has ${bytes(p.free_bytes)}` : ''}.{short ? ' Free some space, or import fewer files at a time.' : ''}</p>
      {#if p.part_errors.length}<p class="small warn">Could not read: {p.part_errors.join('; ')}</p>{/if}
      <Button kind="primary" block disabled={busy || short || p.photos === 0} onclick={() => start(paths)}>{busy ? 'Starting…' : 'Import'}</Button>
      <Button block disabled={busy} onclick={() => { plan = null; paths = []; }}>Choose other files</Button>
    {:else}
      <p>Get your photos out of Google first. This takes Google a few hours to a few days:</p>
      <ol class="small steps">
        <li>Open <a href="https://takeout.google.com" target="_blank" rel="noreferrer">takeout.google.com</a> and choose <b>Deselect all</b>, then tick only <b>Google Photos</b>.</li>
        <li>Choose <b>.zip</b> and the largest file size (50 GB), then <b>Export once</b>.</li>
        <li>When Google emails you, download every part. Then choose them all here, at once.</li>
      </ol>
      <p class="muted small">Your originals stay where they are. Nothing is unpacked next to them; Shoebox reads the files and copies the photos in.</p>
      {#if planError}<p class="small warn">{planError}</p>{/if}
      {#if canPick()}
        <Button kind="primary" block disabled={busy} onclick={async () => choose(await pickArchives().catch((e) => { fail(e); return []; }))}>{busy ? 'Reading…' : 'Choose the Takeout files…'}</Button>
        <Button block disabled={busy} onclick={async () => { const d = await pickFolder('Choose the unpacked Takeout folder').catch((e) => { fail(e); return null; }); if (d) choose([d]); }}>Choose an unpacked folder…</Button>
      {:else}
        <input class="input" placeholder="/Users/you/Downloads/takeout-001.zip, … (comma between files)" bind:value={typed} onkeydown={(e) => e.key === 'Enter' && choose(typed.split(',').map((x: string) => x.trim()).filter(Boolean))} />
        <Button kind="primary" block disabled={busy || !typed.trim()} onclick={() => choose(typed.split(',').map((x: string) => x.trim()).filter(Boolean))}>{busy ? 'Reading…' : 'Look at these files'}</Button>
      {/if}
    {/if}
  </div>
</Sheet>

<style>
  .facts, .steps { margin: 0; padding-left: 18px; display: grid; gap: 4px; }
  .warn { color: var(--risk); }
  .input { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); padding: 0 12px; color: var(--ink); font: inherit; width: 100%; }
</style>
