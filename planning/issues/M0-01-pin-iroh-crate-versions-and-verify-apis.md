---
id: M0-01
title: Pin iroh crate versions and verify APIs
milestone: M0 · Foundations
labels: [area:infra, type:spike]
estimate_days: 1
depends_on: []
features: []
---

# M0-01 · Pin iroh crate versions and verify APIs

## Why

The plan's iroh API names (Endpoint, Router, ProtocolHandler, EndpointAddr, FsStore, iroh-docs) were written
from memory. During planning we couldn't confirm from public sources whether iroh-docs is still maintained or which
iroh / iroh-blobs / iroh-gossip / iroh-docs versions work together. Everything after this builds on that answer.

## Scope

- Find the latest compatible versions of `iroh`, `iroh-blobs`, `iroh-gossip`, `iroh-docs` (crates.io + repos + changelogs).
- Check iroh-docs maintenance status and whether it builds against the pinned iroh.
- Check that local discovery (mDNS), relay config and custom ALPN protocols work as assumed.
- Write `docs/notes/iroh-versions.md`: versions, API names, gaps, links.
- Correct API names in `docs/05-prototype-plan.md`.

## Acceptance criteria

- [ ] Version table committed, with a one-paragraph rationale per crate.
- [ ] A throwaway example (`examples/hello-sync`) connects two endpoints, transfers a blob and syncs one iroh-docs entry.
- [ ] Any blocker (e.g. iroh-docs unmaintained or incompatible) is raised before M0-04 starts, and the plan is adjusted.

## Notes

If iroh-docs is not viable, M1-05 switches to the Loro backend and M1-14 becomes the main backend.

---
_Depends on: none · Estimate: 1d · Features: —_
