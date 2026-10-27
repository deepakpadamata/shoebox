# Shoebox desktop app

Tauri 2 + Svelte 5. Everything the UI can do lives in `src-tauri/src/core.rs` (`AppCore`); the Tauri shell
(`src-tauri/src/main.rs`) and a plain HTTP bridge (`src-tauri/src/http.rs`, feature `http`) are thin adapters over
the same `commands::dispatch`.

```
apps/desktop-mobile/
  src-tauri/   Rust: AppCore, commands, blob serving (shoebox://), keystore, dev bridge, Tauri shell
  ui/          Svelte 5 + Vite + TypeScript (src/bindings is generated from the Rust types)
```

## Work on the UI in a browser (no webview needed)

```bash
cargo run -p desktop-mobile --features http --bin desktop-dev-server -- --data-dir /tmp/shoebox-ui --local-only
cd apps/desktop-mobile/ui && npm install && npm run dev      # http://localhost:1420, proxies to :1421
```
Create a library in the UI, then fill it with `curl -X POST localhost:1421/api/import_paths -d '{"paths":["/path/to/photos"]}'`
or the Import button. `/#/dev` is the component gallery (light and dark).

## Run the desktop app

```bash
cd apps/desktop-mobile/ui && npm install && npm run build
cargo run -p desktop-mobile --features tauri-app --bin shoebox-desktop
# or, with the Tauri CLI:  cargo install tauri-cli --version '^2' && cd src-tauri && cargo tauri dev -- --features tauri-app
```
The device key is kept in the OS keystore (macOS Keychain, Windows Credential Manager, Secret Service); a key
file from an earlier run is moved into it on first launch. Closing the window keeps the app running in the tray
so devices keep syncing; choose *Quit Shoebox* in the tray menu to exit.

## Tests

* `cargo test -p desktop-mobile` — the app core, keystore, blob serving, export; also regenerates
  `ui/src/bindings/*.ts` (CI fails if they changed). Add `--features http` for the HTTP bridge test.
* `cd ui && npm run check && npm test` — svelte-check and the pure UI logic (timeline layout, formatting).

## Two instances in a browser (shared albums)

`.claude/launch.json` has `desktop-ui` (port 1421) and `desktop-ui-b` (1422), two dev servers with separate data
directories. Create a library on each, make a shared album on one, *Invite*, paste the code into *Shared → Join*
on the other: the photos, people and activity show up on both, over loopback.

## Data format and upgrades

The data directory carries `format.json` (see `crates/shoebox-node/src/format.rs`). Opening older data copies the
small files to `backups/format-N-<time>/` and upgrades; opening data from a newer layout is refused with a message.

## Diagnostics

*Settings → Help → Create a diagnostics report* (`diagnostics` command): counts, versions, roles and a redacted
log tail. Nothing is uploaded.

## Not verified

A release `.app` (`cargo tauri build --bundles app --features tauri-app`, unsigned) has run on an Apple-silicon Mac
(R1-08): the key in the Keychain, the device kept across restarts, a second launch refused, sync with a keeper and a
phone. The window itself (rendering, tray, video, file pickers) has been used by the owner but not checked against
the R1-08 list. Installers for other systems, signing, notarisation and auto-update
(`.github/workflows/desktop-release.yml`) have never run: they need signing keys and certificates (R1-14).

## Android (verified on an emulator)

```bash
brew install openjdk@17 && cargo install tauri-cli --version '^2' --locked
export ANDROID_HOME=~/Library/Android/sdk NDK_HOME=$ANDROID_HOME/ndk/<version> JAVA_HOME=/opt/homebrew/opt/openjdk@17
cd apps/desktop-mobile/ui && npm install && npm run build && cd ../src-tauri
cargo tauri android build --debug --apk --target aarch64 --features tauri-app --config '{"build":{"beforeBuildCommand":""}}'
adb install -r gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```
The app core (iroh node, Loro store) runs inside the Android process and the UI renders in the system WebView.
The UI switches to the phone layout on narrow screens. Background backup runs through WorkManager; with the debug build you can trigger it by hand:
`adb shell am broadcast -n org.shoebox.desktop/.BurstNow -a org.shoebox.desktop.BURST_NOW` (watch `adb logcat -s ShoeboxBurst`).
iOS is not built: it needs Xcode (`cargo tauri ios init`), see `planning/PROGRESS.md`.
