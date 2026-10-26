---
id: R1-02
title: Fix the two flaky tests
milestone: R1 · v0.1 testers' preview
labels: [area:store, type:test]
estimate_days: 2
depends_on: []
features: []
---

# R1-02 · Fix the two flaky tests

## Why

CI will run as soon as the repository is uploaded; a red build that is "probably the flake" trains everyone to
ignore it. One of the two flakes is in freeing space, the one place where a bug deletes the only copy of a photo.

## Scope

- `shoebox-store-docs` `autosync::sync_now_waits_for_a_sync_already_in_flight` (about 1 in 6, A-059): find why a sync hangs
  inside iroh-docs, fix it or bound it with a clear error.
- `shoebox-store-docs` `freeup::dry_run_and_real_run_agree_and_pins_are_kept` (2 in 10 before 2026-10-26): find out whether the
  dry run and the real run can disagree in real use (holder confirmations, GC timing) or only in the test. If it is real, fix
  it with a regression test.
- Run each 50 times in a row before calling it fixed.

## Acceptance criteria

- [ ] Both tests pass 50 runs in a row (`for i in $(seq 50); do cargo test -p shoebox-store-docs --test … || break; done`).
- [ ] The cause of each is written down in A-059 (and closed there).
- [ ] If the free-up flake was a real race: a test that reproduces it fails before the fix.

---
_Depends on: — · Estimate: 2d · Features: —_
