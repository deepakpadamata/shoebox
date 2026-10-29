# Shoebox — a decentralized photos app, built on top of iroh

Your photos live on *your* devices (phone, laptop, a box under the desk) and sync directly between them, and with
the people you share with, over [iroh](https://iroh.computer) peer-to-peer connections. No account, no cloud bucket,
no server that owns your library.

> **Status: 0.1, a testers' preview.** It works, and it is early. Keep your photos somewhere else as well while you
> try it, and see [CHANGELOG.md](CHANGELOG.md) for what is new and what is known not to work yet.

- [Install and use Shoebox](#install-and-use-shoebox)
- [Develop on Shoebox](#develop-on-shoebox)

---

## Install and use Shoebox

### What you need

- **Two devices or more.** Shoebox keeps your library in sync *between your devices*; one device on its own is just a
  photo viewer with no backup.
- **Ideally a keeper:** a computer that is always on (a spare PC, a NAS, a Raspberry Pi, a small server) and holds
  every original. It is what makes losing a phone a non-event. See the [keeper guide](docs/guides/keeper-guide.md).

Each device gets a **role** when it joins: a *Keeper* holds every original, a *Companion* (phones, laptops) holds
previews of everything plus your recent photos, favourites and pins, and a *Viewer* holds previews only.

### Download

Everything is on the [releases page](https://github.com/deepakpadamata/shoebox/releases). Each release lists the
SHA-256 checksums of its files.

| Device | File | Install |
|---|---|---|
| **Mac** | `.dmg` | Open it and drag Shoebox to Applications. |
| **Windows** | `.msi` or `-setup.exe` | Run the installer. |
| **Linux desktop** | `.AppImage`, `.deb` or `.rpm` | `chmod +x` and run the AppImage, or install the package with your package manager. |
| **Android** | `.apk` | Open the file on the phone and allow *Install unknown apps* for your browser or file manager when asked. |
| **Keeper** (Linux x86-64 / ARM64, macOS) | `shoebox-keeper-<platform>.tar.gz` | The install script below. |

The keeper's install script downloads the release for your machine, checks its checksum and signature (it needs
[minisign](https://jedisct1.github.io/minisign/): `apt install minisign` / `brew install minisign`), and installs
`shoebox-keeper`:

```bash
curl -fsSL https://raw.githubusercontent.com/deepakpadamata/shoebox/main/scripts/install-keeper.sh | sh
```

New versions install over the old one; your library stays. The app shows a banner when a newer version is out
(*Settings → About*), and the keeper says so in `shoebox-keeper status`. Nothing installs itself.

### First run

1. **Create a library** on your first device, and give it a name and a role.
2. **Save the recovery kit.** You are shown 24 words. They are the only way back if *every* device is lost, so write
   them down and keep them somewhere safe and offline.
3. **Add your other devices.** On a device already in the library, open *Devices → Add a device*, choose the new
   device's role and show the code. On the new device, choose to join an existing library and scan or paste the
   code. Check that **both screens show the same six digits**.
4. **Add a keeper** the same way. On the keeper machine:

   ```bash
   shoebox-keeper service install                # starts on boot (systemd on Linux, launchd on macOS)
   shoebox-keeper pair <the code> --name "Pi in the hall"
   shoebox-keeper status
   ```

5. **Bring photos in.** On a computer, *Import* a folder (or have Shoebox watch one). On a phone, turn on camera
   backup in *Settings*; Wi-Fi-only, charging-only and battery limits are yours to choose.

From then on, every device shows every photo. Each photo says how many devices hold its original: two or more, or
one on a keeper, counts as backed up. *Delete* moves a photo to Recently Deleted on every device for 30 days.

### Learn more

- [User guide](docs/guides/user-guide.md): roles, sharing albums with other people, freeing space, recovery,
  when something goes wrong.
- [Keeper guide](docs/guides/keeper-guide.md): sizing, running it as a service, every `shoebox-keeper` command.
- [Relay guide](docs/guides/relay-guide.md): running your own relay instead of iroh's public ones.

**Reporting a problem:** in the app, *Settings → Help → Create a diagnostics report* produces a redacted report to
attach to an [issue](https://github.com/deepakpadamata/shoebox/issues) (on a keeper, include
`shoebox-keeper status`). Nothing is uploaded on its own.

---

## Develop on Shoebox

### Layout

```
crates/          libraries (Apache-2.0 OR MIT)
  shoebox-core     domain model and merge rules; no async runtime, no iroh
  shoebox-node     the node: sync, blobs, identity, pairing, fetcher, free-up, formats and migrations
  shoebox-store*   library and album stores (Loro for the library, iroh-docs for shared albums, in-memory for tests)
  shoebox-media    import: metadata, thumbnails, previews, location stripping
  shoebox-testkit  multi-node test clusters with partitions and a fake clock
apps/            applications (AGPL-3.0-only)
  desktop-mobile   the app: Tauri 2 + Svelte 5, one code base for desktop and Android
  keeper           shoebox-keeper, the always-on daemon and its CLI
tools/           shoebox-dev (developer CLI), store-bench
xtask/           cargo xtask fixtures | samples
docs/            vision, features, architecture, guides, notes (assumptions, formats), threat model
design/          HTML mockups of every screen
site/            the static website and download page
```

### Prerequisites

- **Rust** through [rustup](https://rustup.rs). `rust-toolchain.toml` pins `stable` with rustfmt and clippy, so
  rustup installs the right toolchain on the first `cargo` command.
- **Node.js 22** and npm, for the app's UI.
- **Python 3**, for the CI scripts (licence notices, dependency guard, site build).
- **Linux only:** the webview and tray libraries the desktop app links against:

  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libdbus-1-dev
  ```

- **Android only:** Android Studio's SDK and NDK, JDK 17 and the Tauri CLI (see [Android](#android) below).

### Build and test

```bash
git clone git@github.com:deepakpadamata/shoebox.git && cd shoebox
cargo build --workspace
cargo test --workspace
cd apps/desktop-mobile/ui && npm install && npm run check && npm test
```

Before pushing, run what CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/licences.py --check
```

`cargo test -p desktop-mobile` also regenerates the TypeScript bindings in `apps/desktop-mobile/ui/src/bindings`;
commit them when they change (CI fails on a diff). `target/` grows quickly, so run `scripts/prune-target.sh` now and
then.

### Work on the UI in a browser

The fastest loop needs no webview. A dev server runs the app core behind HTTP, and Vite serves the UI with hot
reload:

```bash
cargo run -p desktop-mobile --features http --bin desktop-dev-server -- --data-dir /tmp/shoebox-ui --local-only
cd apps/desktop-mobile/ui && npm run dev      # http://localhost:1420
```

Create a library in the UI and import a folder. `/#/dev` is the component gallery. Two dev servers with separate
`--data-dir`s and `--listen` ports act as two devices, which is how pairing and shared albums are tried locally.
`cargo xtask fixtures --preset small|medium|large` makes a synthetic photo corpus to import.

### Run the desktop app

```bash
cd apps/desktop-mobile/ui && npm run build && cd -
cargo run -p desktop-mobile --features tauri-app --bin shoebox-desktop
```

The device key goes into the OS keystore (Keychain, Credential Manager, Secret Service). Closing the window keeps
Shoebox syncing in the tray; *Quit Shoebox* in the tray menu exits.

### Android

```bash
brew install openjdk@17 && cargo install tauri-cli --version '^2' --locked
export ANDROID_HOME=~/Library/Android/sdk NDK_HOME=$ANDROID_HOME/ndk/<version> JAVA_HOME=/opt/homebrew/opt/openjdk@17
cd apps/desktop-mobile/ui && npm run build && cd ../src-tauri
cargo tauri android build --debug --apk --target aarch64 --features tauri-app --config '{"build":{"beforeBuildCommand":""}}'
adb install -r gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

More on the app (background backup, the HTTP bridge, data format) is in
[apps/desktop-mobile/README.md](apps/desktop-mobile/README.md).

### Run a keeper from source

```bash
cargo run -p keeper -- run --storage /tmp/keeper-a --name "dev keeper"
cargo run -p keeper -- --storage /tmp/keeper-a status
```

`shoebox-dev` drives nodes from the command line without the app; `docs/notes/demo-m1.sh` walks through three
devices on one machine.

### Read before changing things

- [CONTRIBUTING.md](CONTRIBUTING.md): conventions, and the rules for **changing a wire protocol or on-disk format**.
  From 0.1 on, people's devices hold data that every later build must read.
- [docs/03-architecture.md](docs/03-architecture.md): how the features map onto iroh (endpoints, blobs, docs,
  gossip).
- [docs/notes/assumptions.md](docs/notes/assumptions.md): the decisions made while building, by number (A-0xx), which
  commit messages refer to.
- [docs/security/threat-model.md](docs/security/threat-model.md).

## Licence

Libraries (`crates/`, `tools/`, `xtask/`, `examples/`) are `Apache-2.0 OR MIT`; the apps (`apps/`) are
`AGPL-3.0-only`. See [LICENSE-APACHE](LICENSE-APACHE), [LICENSE-MIT](LICENSE-MIT) and
[apps/LICENSE-AGPL](apps/LICENSE-AGPL).
