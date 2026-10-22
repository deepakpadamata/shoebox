<script lang="ts">
  // The recovery kit is shown once after creating a library (and its reminder reopens this view
  // only to confirm it was saved: the phrase itself is not stored anywhere on this device).
  import { api } from '../lib/api';
  import { app, fail, refreshState, toast } from '../lib/store.svelte';
  import Button from '../components/Button.svelte';

  interface Props { phrase?: string; kitText?: string; onsaved: () => void; }
  let { phrase, kitText, onsaved }: Props = $props();
  const words = $derived(phrase ? phrase.split(' ') : []);
  let confirmed = $state(false);

  function download() {
    if (!kitText) return;
    const url = URL.createObjectURL(new Blob([kitText], { type: 'text/plain' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'shoebox-recovery-kit.txt';
    a.click();
    URL.revokeObjectURL(url);
  }

  async function copy() {
    if (phrase) await navigator.clipboard.writeText(phrase).then(() => toast('Phrase copied. Paste it somewhere safe, then clear the clipboard.')).catch(fail);
  }

  async function done() {
    try {
      await api.markRecoveryKitSaved();
      await refreshState();
      onsaved();
    } catch (e) {
      fail(e);
    }
  }
</script>

<div class="stack">
  {#if phrase}
    <p class="small muted">These 24 words are the only way to get back into this library if every device is lost. Anyone with them can take it over. Print them or store them in a password manager, not next to your photos.</p>
    <ol class="words">
      {#each words as w, i}<li><span class="n">{i + 1}</span>{w}</li>{/each}
    </ol>
    <div class="row-flex">
      <Button size="sm" onclick={download}>⤓ Save as file</Button>
      <Button size="sm" onclick={copy}>⧉ Copy</Button>
      <Button size="sm" onclick={() => window.print()}>🖨 Print</Button>
    </div>
    <label class="row-flex small"><input type="checkbox" bind:checked={confirmed} /> I have saved my recovery kit somewhere safe</label>
    <Button kind="primary" block disabled={!confirmed} onclick={done}>Continue</Button>
  {:else}
    <p class="small muted">The recovery phrase is only shown when a library is created and is not kept on this device. If you saved it, confirm here and this reminder goes away. If you did not, ask another device to show it, or note that a new library would need to be created.</p>
    <label class="row-flex small"><input type="checkbox" bind:checked={confirmed} /> I have my recovery kit saved</label>
    <Button kind="primary" block disabled={!confirmed} onclick={done}>Confirm</Button>
  {/if}
</div>

<style>
  .words { list-style: none; padding: 0; margin: 0; display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; }
  .words li { background: var(--surface-2); border-radius: 9px; padding: 7px 10px; font: 600 13px var(--mono); }
  .n { color: var(--ink-3); display: inline-block; width: 22px; font-weight: 400; }
  @media print { :global(body) * { visibility: hidden; } .words, .words * { visibility: visible; } .words { position: absolute; left: 0; top: 0; } }
</style>
