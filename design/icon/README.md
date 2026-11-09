# App icon

Two tilted Polaroid prints woven through each other. The sun is in one print and the hills in the other, so the picture is only whole across both: one library spread over your devices.

- `shoebox-icon.svg`: the main icon (macOS-style tile with padding, used for desktop)
- `shoebox-icon-ios.svg`: the same prints full-bleed on a square, opaque background (iOS rounds the corners itself and shows transparency as black); the iOS icons in `apps/desktop-mobile/src-tauri/icons/ios` come from it
- `shoebox-icon-fg.svg` / `shoebox-icon-bg.svg`: Android adaptive icon layers
- `shoebox-icon-mono.svg`: single colour, for Android themed icons (and later the tray / notification icon)

After editing an SVG, re-render the PNGs and regenerate every platform's icons:

```bash
cd design/icon && for f in shoebox-icon shoebox-icon-bg shoebox-icon-fg shoebox-icon-mono; do rsvg-convert -w 1024 -h 1024 $f.svg -o $f.png; done
cd ../../apps/desktop-mobile/src-tauri && cargo tauri icon ../../../design/icon/icon-manifest.json
```

The iOS set comes from its own picture (`cargo tauri icon ../../../design/icon/shoebox-icon-ios.png -o <dir>`, then remove the alpha channel from every PNG in `<dir>/ios` and copy them to `icons/ios` and to `gen/apple/Assets.xcassets/AppIcon.appiconset`).
