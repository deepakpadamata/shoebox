# App icon

Two tilted Polaroid prints woven through each other. The sun is in one print and the hills in the other, so the picture is only whole across both: one library spread over your devices.

- `shoebox-icon.svg`: the main icon (macOS-style tile with padding, used for desktop and iOS)
- `shoebox-icon-fg.svg` / `shoebox-icon-bg.svg`: Android adaptive icon layers
- `shoebox-icon-mono.svg`: single colour, for Android themed icons (and later the tray / notification icon)

After editing an SVG, re-render the PNGs and regenerate every platform's icons:

```bash
cd design/icon && for f in shoebox-icon shoebox-icon-bg shoebox-icon-fg shoebox-icon-mono; do rsvg-convert -w 1024 -h 1024 $f.svg -o $f.png; done
cd ../../apps/desktop-mobile/src-tauri && cargo tauri icon ../../../design/icon/icon-manifest.json
```
