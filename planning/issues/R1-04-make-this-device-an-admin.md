---
id: R1-04
title: Make this device an admin
milestone: R1 · v0.1 testers' preview
labels: [area:app, type:feature]
estimate_days: 1
depends_on: []
features: []
---

# R1-04 · Make this device an admin

## Why

Usually only the device that created the library is an admin (pairing never makes one, A-062). If it is lost,
every role change and removal needs the recovery phrase. A second admin, chosen on purpose, avoids that.

## Scope

- Node: `make_admin_with_phrase(phrase, device)` issues a root-signed admin cert (the other devices alert on it, A-087).
- Devices screen: "Make admin" on a device that is not one, asking for the phrase; the row shows "admin" after.
- Removing an admin still needs the phrase (A-085).

## Acceptance criteria

- [ ] Test: a phone made admin can pair a device and change a role without the phrase.
- [ ] Wrong phrase changes nothing.
- [ ] Checked in two dev instances; the other device shows the new-admin alert.

---
_Depends on: — · Estimate: 1d · Features: —_
