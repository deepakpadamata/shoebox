---
id: M3-07
title: Desktop onboarding and recovery-kit reminder
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 3
depends_on: [M3-06, M2-02, M2-04]
features: [F1.2, F1.3, F1.4, F1.5]
---

# M3-07 · Desktop onboarding and recovery-kit reminder

## Why

First-run flows from mockups 1a–1d, adapted for desktop, plus the persistent reminder (2d).

## Scope

- Welcome: start new library / join existing (shows a ticket + scans nothing on desktop; desktop joins by pasting a ticket or showing its own QR for the phone to scan).
- Name and avatar; recovery kit save (file + show phrase); skip leads to the reminder.
- Import source (folders) and role selection (default Keeper on desktop).
- Recovery-kit banner with 'Remind me tomorrow' and the avatar dot; banner priority rule (paused > at-risk > recovery).

## Acceptance criteria

- [ ] A new user goes from launch to seeing imported photos in under 2 minutes.
- [ ] The banner reappears the day after 'Remind me tomorrow' and disappears permanently once the kit is saved.

---
_Depends on: M3-06, M2-02, M2-04 · Estimate: 3d · Features: F1.2, F1.3, F1.4, F1.5_
