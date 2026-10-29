<script lang="ts">
  import { onMount } from 'svelte';
  import { app, start } from './lib/store.svelte';
  import { section } from './lib/route.svelte';
  import Onboarding from './screens/Onboarding.svelte';
  import Shell from './screens/Shell.svelte';
  import MobileShell from './screens/MobileShell.svelte';
  import PairingPrompt from './screens/PairingPrompt.svelte';
  import { viewport } from './lib/viewport.svelte';
  import Components from './dev/Components.svelte';
  import Logo from './components/Logo.svelte';

  onMount(start);
</script>

{#if section() === 'dev'}
  <Components />
{:else if app.loading}
  <div class="splash" role="status" aria-label="Opening your library">
    <span class="logo"><Logo size={88} /></span>
  </div>
{:else if app.error}
  <div class="splash"><b>Shoebox could not start</b><span class="muted">{app.error}</span></div>
{:else if !app.state?.has_library}
  <Onboarding />
{:else if viewport.mobile}
  <MobileShell />
{:else}
  <Shell />
{/if}

{#if app.state?.has_library}<PairingPrompt />{/if}

{#if app.toast}<div class="toast" role="status">{app.toast}</div>{/if}

<style>
  .splash { height: 100%; display: grid; place-content: center; text-align: center; gap: 6px; color: var(--ink-2); }
  /* The app icon (design/icon/shoebox-icon.svg), fading in so a quick start shows no flash. */
  .logo { display: block; animation: appear 0.4s ease-out 0.15s both, breathe 2.4s ease-in-out 0.6s infinite; }
  @keyframes appear { from { opacity: 0; transform: scale(0.94); } }
  @keyframes breathe { 50% { transform: scale(0.96); } }
  @media (prefers-reduced-motion: reduce) { .logo { animation: appear 0.4s ease-out 0.15s both; } }
  .toast { position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); background: var(--ink); color: var(--bg);
    padding: 10px 16px; border-radius: 12px; box-shadow: var(--shadow); z-index: 100; max-width: 80vw; }
</style>
