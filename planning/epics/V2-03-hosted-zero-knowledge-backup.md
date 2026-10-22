---
id: V2-03
title: Hosted zero-knowledge cloud backup
milestone: v2
labels: [epic, area:infra, hosted, area:security]
features: [F7.7]
---

# V2-03 · Hosted zero-knowledge cloud backup

## Goal
An optional paid service that acts as a keeper holding only ciphertext. It counts toward copy count. Decision #6 + follow-up.

## Likely scope
- Client-side encryption of blobs and metadata snapshots with a library backup key (derived from the root key, stored in the recovery kit).
- The service speaks the blobs protocol for encrypted blobs. Holdings are tagged "encrypted copy" so copy-count rules can decide whether it counts toward the redundancy target.
- Restore flow: new device + recovery kit → pull ciphertext → decrypt locally.
- Billing, quotas, account-less payment (subscription tied to a library id + payment token).

## Open questions
- Pricing and payment method without accounts.
- Does an encrypted copy count fully toward the redundancy target?
- Dedupe across users is impossible by design (and that's intended).

## Depends on
V2-04, MVP.
