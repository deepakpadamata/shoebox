# Release signing

Every download is built by `.github/workflows/release.yml` from a `vX.Y.Z` tag (`vX.Y.Z-rc.N` for a pre-release)
into one draft GitHub release. The workflow's first job refuses to build while any secret below is missing or
`scripts/install-keeper.sh` still carries the placeholder key. Keys are made once, by the owner, on their own machine;
the secret halves go into *Settings → Secrets and variables → Actions* and a password manager, never into the
repository.

| What | Signed with | Checked by |
|---|---|---|
| Keeper tarballs, and every other file through `SHA256SUMS` | minisign | `install-keeper.sh`, anyone with the public key |
| macOS app (`.dmg`) | Apple Developer ID, notarised | Gatekeeper |
| Android APK | the project's release keystore | Android, on install and on every update |
| Windows installers | not signed yet (SmartScreen warns on first run) | — |
| Linux AppImage / `.deb` / `.rpm` | through `SHA256SUMS` only | — |

**Losing a key.** The Android keystore cannot be replaced: an APK signed with another key does not install over
the old one, so testers would have to uninstall (and lose the app's data folder). Keep two offline copies. A lost
minisign key means a new public key in `install-keeper.sh` and telling keeper users; a lost Apple certificate is
reissued from the developer account.

## minisign (keeper and checksums)

```bash
brew install minisign            # or: apt install minisign
minisign -G -p shoebox.pub -s shoebox.key     # asks for a password
```

- `MINISIGN_SECRET_KEY`: the whole content of `shoebox.key`.
- `MINISIGN_PASSWORD`: its password.
- The public key (the second line of `shoebox.pub`, starting `RW`) replaces `RWQ_REPLACE_WITH_THE_PROJECT_PUBLIC_KEY`
  in `scripts/install-keeper.sh`, in a commit.

## Apple (macOS app)

Needs a paid Apple Developer Program membership.

1. In Xcode → *Settings → Accounts → Manage Certificates*, or at developer.apple.com, create a **Developer ID
   Application** certificate. Export it from Keychain Access as a `.p12` with a password.
2. Make an app-specific password for your Apple ID at appleid.apple.com (*Sign-In and Security*).
3. Set the secrets:
   - `APPLE_CERTIFICATE`: `base64 -i developer-id.p12 | pbcopy`
   - `APPLE_CERTIFICATE_PASSWORD`: the `.p12` export password
   - `APPLE_SIGNING_IDENTITY`: the certificate's name, as `security find-identity -v -p codesigning` prints it
     (`Developer ID Application: Your Name (TEAMID)`)
   - `APPLE_ID`: your Apple ID email; `APPLE_PASSWORD`: the app-specific password; `APPLE_TEAM_ID`: the ten-character
     team id

## Android (APK)

```bash
keytool -genkeypair -v -keystore shoebox-release.jks -alias shoebox -keyalg RSA -keysize 4096 -validity 10000
```

- `ANDROID_KEYSTORE_BASE64`: `base64 -i shoebox-release.jks | pbcopy`
- `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_PASSWORD`: the passwords `keytool` asked for (the same one twice is fine)
- `ANDROID_KEY_ALIAS`: `shoebox`

To build a signed APK locally, put the keystore next to the Android project and describe it in
`apps/desktop-mobile/src-tauri/gen/android/keystore.properties` (ignored by git):

```properties
storeFile=shoebox-release.jks
storePassword=…
keyAlias=shoebox
keyPassword=…
```

Without that file, `cargo tauri android build` makes an unsigned release APK and debug builds use the debug key, as
before. A phone with a debug build must uninstall it before installing a release build (different keys).

## Cutting a release

```bash
git tag v0.1.0-rc.1 && git push origin v0.1.0-rc.1
```

The tag must match `version` in `Cargo.toml` (an `-rc.N` suffix is allowed). When the run is green, open the draft
release, check the files, write the notes from `CHANGELOG.md` and publish it. `install-keeper.sh` takes
`SHOEBOX_VERSION=v0.1.0-rc.1` for a pre-release; without it, it installs the latest full release.
