---
id: V2-01
title: Self-hosted web viewer on a keeper
milestone: v2
labels: [epic, area:keeper, area:ui]
features: [F6.8]
---

# V2-01 · Self-hosted web viewer on a keeper

## Goal
Let a keeper serve a read-only web page for a shared album, so people without the app can view it. Decision #6.

## Likely scope
- Keeper flag `album publish <name>`, which serves an HTTP(S) view of the album's previews (originals optional).
- Link-based access with an expiring token; no accounts.
- Reachability: the user's own domain/port-forward, or tunnelled through the hosted gateway (V2-02).

## Open questions
- TLS story for home keepers (via the gateway, Tailscale-style, or bring your own).
- Does the web view count as a "member" in album limits?

## Depends on
MVP (M5 shared albums, M2 keeper).
