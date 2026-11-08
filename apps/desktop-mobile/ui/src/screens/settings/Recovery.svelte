<script lang="ts">
  import { api } from '../../lib/api';
  import { app, fail, toast } from '../../lib/store.svelte';
  import Button from '../../components/Button.svelte';
  import Icon from '../../components/Icon.svelte';
  import Sheet from '../../components/Sheet.svelte';
  import RecoveryKit from '../RecoveryKit.svelte';

  let kitOpen = $state(false);
</script>

<div class="set-section">Recovery kit</div>
<div class="card pad small stack">
  {#if app.state?.recovery_kit_saved}<span><Icon name="check" size={16} /> You confirmed the recovery kit is saved.</span>{:else}<span>The recovery kit has not been confirmed as saved.</span>{/if}
  <div class="muted">The kit is the 24 words that bring your library back if every device is lost. Only you have them: Shoebox does not keep a copy, and nobody can recover it for you.</div>
  {#if !app.state?.recovery_kit_saved}<div><Button size="sm" kind="primary" onclick={() => (kitOpen = true)}>I saved it</Button></div>{/if}
  <div class="muted">If every other device is lost, a new device recovers the library with the 24 words plus this device's link.</div>
  <div><Button size="sm" onclick={async () => { try { const l = await api.deviceLink(); await navigator.clipboard.writeText(l); toast('This device’s link was copied'); } catch (e) { fail(e); } }}>Copy this device’s link</Button></div>
</div>

<Sheet open={kitOpen} title="Recovery kit" onclose={() => (kitOpen = false)} width={520}>
  <RecoveryKit onsaved={() => (kitOpen = false)} />
</Sheet>

<style>
  .pad { padding: 14px; }
</style>
