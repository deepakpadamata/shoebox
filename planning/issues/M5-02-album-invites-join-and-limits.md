---
id: M5-02
title: Album invites, join and limits
milestone: M5 · Shared albums
labels: [area:node, area:security, type:feature]
estimate_days: 3
depends_on: [M5-01, M2-04]
features: [F6.1, F6.2]
---

# M5-02 · Album invites, join and limits

## Why

Invites are bearer tickets scoped to one album with view or add rights (mockups 4c, 4d).

## Scope

- `InviteTicket{album, capability: Read|Write, addr hints, secret, expires, uses}`; single-use and 7-day default.
- Join flow reusing the pair handshake without SAS; the joiner gets the album capability and a `Member` entry.
- Joining without a library creates a minimal library (shared-albums-only mode).
- Limits (decision #19): 50 members, 50k items. Inviting or adding past the limit is blocked with a clear message.

## Acceptance criteria

- [ ] View-only members can't add items (enforced on every replica, not just in the UI).
- [ ] An expired or used ticket fails with a specific error.

---
_Depends on: M5-01, M2-04 · Estimate: 3d · Features: F6.1, F6.2_
