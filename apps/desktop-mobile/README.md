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

## Not verified

The Tauri window itself (rendering, tray, keystore prompts, single-instance behaviour) has only been
**compiled** here; the UI was exercised in a browser against the dev bridge. Installers, signing and auto-update
(`.github/workflows/desktop-release.yml`) have never run: they need signing keys and certificates.
