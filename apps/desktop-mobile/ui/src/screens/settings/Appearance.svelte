<script lang="ts">
  import { applyTextSize, savedTextSize } from '../../lib/textsize';
  import { applyTheme, savedTheme } from '../../lib/theme';
  import Segmented from '../../components/Segmented.svelte';

  let theme = $state(savedTheme());
  let textSize = $state(savedTextSize());
  function chooseTheme(v: string) {
    theme = v;
    applyTheme(v);
  }
  function setTextSize(v: string) {
    textSize = v;
    try { localStorage.setItem('textSize', v); } catch { /* not remembered */ }
    applyTextSize(v);
  }
</script>

<div class="set-section">Appearance</div>
<div class="card pad">
  <Segmented value={theme} options={[{ value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }]} onchange={chooseTheme} />
  <div class="small muted" style="margin:12px 0 6px">Text size</div>
  <Segmented value={textSize} options={[{ value: 'small', label: 'Small' }, { value: 'default', label: 'Default' }, { value: 'large', label: 'Large' }, { value: 'xlarge', label: 'Largest' }]} onchange={setTextSize} />
</div>

<style>
  .pad { padding: 14px; }
</style>
