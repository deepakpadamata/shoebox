<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type AboutInfo } from '../../lib/api';
  import { fail, toast } from '../../lib/store.svelte';
  import Button from '../../components/Button.svelte';
  import Sheet from '../../components/Sheet.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import { cfg, loadSettings } from './store.svelte';

  let about = $state<AboutInfo | null>(null);
  let licences = $state<string | null>(null);
  let checking = $state(false);
  onMount(() => {
    loadSettings();
    api.about().then((a) => (about = a)).catch(() => {});
  });
  const s = $derived(cfg.s);
  async function checkNow() {
    checking = true;
    try {
      const u = await api.checkForUpdate();
      toast(u ? `Shoebox ${u.version} is available` : 'You have the latest version');
    } catch (e) {
      fail(e);
    } finally {
      checking = false;
    }
  }
</script>

<div class="set-section">About</div>
<div class="card pad small stack">
  {#if about}
    <div><b>Shoebox {about.version}</b> <span class="muted mono">({about.commit})</span></div>
    <div class="muted">Shoebox is free software under the GNU Affero General Public License 3.0. You can get its source code, and the source of this exact version, at <a href={about.source_url} target="_blank" rel="noreferrer">{about.source_url}</a>.</div>
  {/if}
  <div><Button size="sm" onclick={async () => { try { licences = await api.thirdPartyLicences(); } catch (e) { fail(e); } }}>Third-party licences</Button></div>
  {#if s}
    <label class="row-flex"><Toggle checked={s.update_checks} label="Check for new versions" onchange={async (v) => { try { await api.setUpdateChecks(v); if (s) s.update_checks = v; } catch (e) { fail(e); } }} /> Check for new versions</label>
    <div class="muted">Twice a day the app reads the version number on the Shoebox website. Nothing about you or your library is sent; the website sees only that someone asked.</div>
    <div><Button size="sm" disabled={checking} onclick={checkNow}>{checking ? 'Checking…' : 'Check now'}</Button></div>
  {/if}
</div>

<Sheet open={licences !== null} title="Third-party licences" onclose={() => (licences = null)} width={720}>
  {#if licences !== null}
    <textarea class="report mono" readonly rows="22" value={licences}></textarea>
  {/if}
</Sheet>

<style>
  .pad { padding: 14px; }
  .report { width: 100%; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); color: var(--ink); font-size: var(--t-xs); padding: 10px; }
</style>
