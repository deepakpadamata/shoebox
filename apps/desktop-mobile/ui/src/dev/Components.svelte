<script lang="ts">
  // The component gallery (M3-05): every building block of the mockups, in light and dark.
  import Avatar from '../components/Avatar.svelte';
  import Banner from '../components/Banner.svelte';
  import Button from '../components/Button.svelte';
  import List from '../components/List.svelte';
  import Pill from '../components/Pill.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import QR from '../components/QR.svelte';
  import Row from '../components/Row.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Sheet from '../components/Sheet.svelte';
  import Tile from '../components/Tile.svelte';
  import Toggle from '../components/Toggle.svelte';

  let on = $state(true);
  let seg = $state('b');
  let sheet = $state(false);
  const attr = document.documentElement.getAttribute('data-theme');
  let dark = $state(attr ? attr === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches);
  const qr = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><path d="M0 0h7v7H0zM14 0h7v7h-7zM0 14h7v7H0z" fill="#000"/><path d="M2 2h3v3H2zM16 2h3v3h-3zM2 16h3v3H2z" fill="#fff"/><path d="M9 1h2v2H9zM9 5h4v2H9zM8 9h3v2H8zM12 9h2v4h-2zM16 10h4v2h-4zM9 14h2v3H9zM13 15h3v2h-3zM17 14h3v3h-3zM9 18h3v2H9z" fill="#000"/></svg>';
  function theme(v: boolean) {
    dark = v;
    if (v) document.documentElement.setAttribute('data-theme', 'dark');
    else document.documentElement.setAttribute('data-theme', 'light');
  }
  // Tiles use blob URLs that do not exist on this page; the placeholder state is what is shown.
  const fake = '0'.repeat(64);
</script>

<div class="gallery">
  <header class="row-flex">
    <h1 class="grow">Components</h1>
    <span class="small muted">Dark</span><Toggle checked={dark} label="Dark mode" onchange={theme} />
  </header>

  <section><h2>Buttons</h2>
    <div class="row-flex"><Button>Default</Button><Button kind="primary">Primary</Button><Button kind="ghost">Ghost</Button><Button kind="danger">Danger</Button><Button size="sm">Small</Button><Button disabled>Disabled</Button></div>
  </section>

  <section><h2>Pills &amp; badges</h2>
    <div class="row-flex"><Pill tone="ok">2 copies</Pill><Pill tone="warn">Via relay</Pill><Pill tone="risk">1 copy</Pill><Pill tone="info">Transferring</Pill><Pill>v1.x</Pill></div>
  </section>

  <section><h2>Tiles</h2>
    <div class="row-flex">
      <Tile id={fake} size={110} hasThumb={false} />
      <Tile id={fake} size={110} hasThumb={false} favorite copies={1} />
      <Tile id={fake} size={110} hasThumb={false} video transferring />
      <Tile id={fake} size={110} hasThumb={false} selecting selected />
      <Tile id={fake} size={110} hasThumb={false} selecting />
    </div>
  </section>

  <section><h2>List &amp; rows</h2>
    <List>
      <Row icon="📁" title="Watched folder" subtitle="/Users/you/Pictures">{#snippet trail()}<span class="chev">›</span>{/snippet}</Row>
      <Row title="MacBook Pro">{#snippet lead()}<Avatar name="MacBook Pro" />{/snippet}{#snippet trail()}<Pill tone="ok">Direct</Pill>{/snippet}</Row>
      <Row title="Pi keeper" subtitle="keeper · 12 ms">{#snippet lead()}<Avatar name="Pi" />{/snippet}</Row>
    </List>
  </section>

  <section><h2>Toggle &amp; segmented</h2>
    <div class="row-flex"><Toggle bind:checked={on} label="Example" /><span class="small muted">{on ? 'On' : 'Off'}</span></div>
    <div style="max-width:360px;margin-top:10px"><Segmented bind:value={seg} options={[{ value: 'a', label: 'Keeper' }, { value: 'b', label: 'Phone' }, { value: 'c', label: 'Viewer' }]} /></div>
  </section>

  <section><h2>Avatars</h2>
    <div class="row-flex"><Avatar name="Ana" /><Avatar name="Diego" /><Avatar name="Mei" size={44} /><span class="avatars"><Avatar name="A" /><Avatar name="B" /><Avatar name="C" /></span></div>
  </section>

  <section><h2>Progress</h2>
    <div style="max-width:420px" class="stack">
      <ProgressBar total={100} segments={[{ value: 60, tone: 'ok' }, { value: 25, tone: 'warn' }, { value: 5, tone: 'risk' }]} label="Copies" />
      <ProgressBar total={100} segments={[{ value: 35, tone: 'info' }]} label="Transferring" />
    </div>
  </section>

  <section><h2>Banners</h2>
    <div class="stack" style="max-width:560px">
      <Banner tone="risk" action="View"><b>212 photos</b> are only on this phone.</Banner>
      <Banner tone="warn" icon="◌">Backup is paused: nothing is reachable.</Banner>
      <Banner tone="accent" icon="🔑" ondismiss={() => {}}>Save your recovery kit.</Banner>
      <Banner tone="info" icon="✓">Everything has two copies.</Banner>
    </div>
  </section>

  <section><h2>Sheet &amp; QR</h2>
    <div class="row-flex"><Button onclick={() => (sheet = true)}>Open sheet</Button><QR svg={qr} size={120} /></div>
    <Sheet open={sheet} title="A sheet" onclose={() => (sheet = false)} width={360}><p class="muted small">Modal content goes here.</p><Button block kind="primary" onclick={() => (sheet = false)}>Done</Button></Sheet>
  </section>
</div>

<style>
  .gallery { height: 100%; overflow: auto; padding: 24px 28px 60px; background: var(--bg); }
  section { margin-top: 24px; }
  h1 { margin: 0; font-size: 24px; } h2 { font-size: 13px; text-transform: uppercase; letter-spacing: .05em; color: var(--ink-2); margin: 0 0 10px; }
  .chev { color: var(--ink-3); }
  .avatars { display: flex; } .avatars :global(.avatar) { border: 2px solid var(--bg); margin-left: -8px; }
</style>
