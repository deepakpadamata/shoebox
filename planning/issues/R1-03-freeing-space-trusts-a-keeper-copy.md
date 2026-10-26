---
id: R1-03
title: Freeing space trusts a keeper copy
milestone: R1 · v0.1 testers' preview
labels: [area:node, type:feature]
estimate_days: 1
depends_on: []
features: []
---

# R1-03 · Freeing space trusts a keeper copy

## Why

Since 2026-10-23 a photo held by one keeper counts as backed up, but freeing space on a phone still needs two other
copies (A-064). A tester with a phone and a keeper sees "backed up" and still cannot free anything, and will assume a bug.

## Scope

- Free-up accepts an original held by a confirmed keeper (a `shoebox/has/0` answer within 7 days) as enough, like the
  backed-up rule; otherwise the existing rule (target copies elsewhere) applies.
- Pins, shared-album references and keepers themselves still never free.
- Storage screen and user guide say the same rule in the same words.

## Acceptance criteria

- [ ] Test: phone + one confirmed keeper frees the phone's originals; phone + one unconfirmed keeper frees nothing.
- [ ] Dry run and real run agree (existing test extended).
- [ ] A-064 updated; user guide's free-up sentence matches the app.

## Notes

If you would rather keep the stricter rule, the issue becomes: explain it on the Storage screen instead.

---
_Depends on: — · Estimate: 1d · Features: —_
