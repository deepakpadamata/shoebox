---
id: M5-05
title: Member management and removal
milestone: M5 · Shared albums
labels: [area:node, area:security, type:feature]
estimate_days: 3
depends_on: [M5-02]
features: [F6.3]
---

# M5-05 · Member management and removal

## Why

Removing someone has to actually stop their future access. With bearer capabilities that means re-keying.

## Scope

- Members list with roles; leave album.
- Remove member (owner/admins of the album): create a new album space, migrate items/events, re-invite the remaining members automatically over authenticated connections.
- The removed member keeps what they already downloaded (stated clearly in the UI).

## Acceptance criteria

- [ ] Testkit: after removal, the removed member gets no new items; everyone else continues in the new space without manual steps.

---
_Depends on: M5-02 · Estimate: 3d · Features: F6.3_
