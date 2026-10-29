import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { applyTextSize, savedTextSize } from './lib/textsize';

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

applyTextSize(savedTextSize());

// No webview menu (Reload, Inspect…) on right-click, except where there is text to copy or edit.
// Development builds keep it for the inspector.
if (import.meta.env.PROD) {
  addEventListener('contextmenu', (e) => {
    if ((e.target as Element | null)?.closest?.('input, textarea, [contenteditable], .mono, .selectable')) return;
    e.preventDefault();
  });
}

export default mount(App, { target: document.getElementById('app')! });
