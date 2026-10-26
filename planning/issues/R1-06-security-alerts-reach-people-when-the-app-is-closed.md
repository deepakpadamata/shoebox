---
id: R1-06
title: Security alerts reach people when the app is closed
milestone: R1 · v0.1 testers' preview
labels: [area:app, type:feature]
estimate_days: 1.5
depends_on: []
features: []
---

# R1-06 · Security alerts reach people when the app is closed

## Why

A new admin or a replaced recovery phrase is the sign that someone has the phrase (A-087, A-088). Today the alert
waits until the app is opened, and the keeper only writes it to its log.

## Scope

- Android: the background worker posts a system notification for `AdminAdded` and `RecoveryPhraseReplaced`.
- Desktop: a system notification when the window is hidden (tray).
- Keeper: `shoebox-keeper status` lists unacknowledged alerts; `shoebox-keeper alerts ack`.

## Acceptance criteria

- [ ] Android emulator: a new admin made while the app is closed shows a notification after the next background run.
- [ ] Keeper status shows the alert until acknowledged.
- [ ] Notification text never contains the phrase or device keys.

---
_Depends on: — · Estimate: 1.5d · Features: —_
