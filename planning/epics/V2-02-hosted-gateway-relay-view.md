---
id: V2-02
title: Hosted gateway "relay view"
milestone: v2
labels: [epic, area:infra, hosted]
features: [F6.8]
---

# V2-02 · Hosted gateway "relay view"

## Goal
An optional service we run that renders a shared album on the web for people without the app. Decision #6.

## Likely scope
- The gateway joins as a read-only album member, or proxies from an online member device or keeper.
- Album read keys carried in the share link's URL fragment, so the server can't read the album without the link.
- Abuse controls: rate limits, takedown process, link revocation.

## Open questions (from 04 "Still open")
- Source of truth: fetch from online members vs hold read keys. The cloud backup (V2-03) is zero-knowledge, so it can't serve.
- Business model / limits for the free tier.
- How viewers are authenticated beyond possessing the link.

## Depends on
V2-04 (own relays), MVP shared albums.
