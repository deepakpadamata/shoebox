---
id: R1-13
title: Signed release builds
milestone: R1 · v0.1 testers' preview
labels: [area:infra, type:chore, needs:owner]
estimate_days: 2
depends_on: [R1-08]
features: []
---

# R1-13 · Signed release builds

## Why

Unsigned macOS apps are blocked by Gatekeeper, Android needs a release key, and the keeper's install script refuses
anything without a valid minisign signature (A-065).

## Scope

- **Owner:** Apple Developer ID (signing and notarisation), an Android release keystore, a minisign key pair; add them as
  repository secrets.
- Put the real minisign public key into the install script.
- Tag `v0.1.0-rc.1` and let the workflows build: macOS universal (notarised), Android APK, keeper for x86_64/aarch64/macOS.
- Install each artifact from scratch on a clean machine.

## Acceptance criteria

- [ ] The macOS app opens without a Gatekeeper warning.
- [ ] The APK installs on a phone that has never had a dev build.
- [ ] `install-keeper.sh` verifies the real signature and installs on the Pi.

---
_Depends on: R1-08 · Estimate: 2d · Features: —_
