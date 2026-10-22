import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

// The Android WebView does not report the status and navigation bars through env(): the shell
// exposes them (`ShoeboxInsets`, see MainActivity.kt) and they become the --safe-* variables.
type Insets = { top(): number; bottom(): number };
function applyInsets() {
  const native = (window as unknown as { ShoeboxInsets?: Insets }).ShoeboxInsets;
  if (!native) return;
  const root = document.documentElement.style;
  root.setProperty('--safe-top', `${native.top()}px`);
  root.setProperty('--safe-bottom', `${native.bottom()}px`);
}
applyInsets();
addEventListener('resize', applyInsets);
setTimeout(applyInsets, 300);

export default mount(App, { target: document.getElementById('app')! });
