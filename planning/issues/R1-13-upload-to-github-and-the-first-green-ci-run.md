---
id: R1-13
title: Upload to GitHub and the first green CI run
milestone: R1 · v0.1 testers' preview
labels: [area:infra, type:chore, needs:owner]
estimate_days: 0.5
depends_on: [R1-01, R1-02, R1-12]
features: []
---

# R1-13 · Upload to GitHub and the first green CI run

## Why

The code goes to GitHub once everything else for 0.1 is done. There has never been a remote, so CI, the nightly
network test and the release workflows have never run (A-006, A-065).

## Scope

- **Owner:** create the repository and give its `OWNER/NAME`.
- Before pushing: run the CI steps locally once more (fmt, clippy, tests, license guard, TS-bindings diff, site check).
- Replace the placeholders: `repository` in `[workspace.package]` (A-002), `OWNER/shoebox` in workflows and the install
  script, the updater feed URL (R1-07).
- Push, make CI green, switch on the nightly real-network workflow (needs the VPS or a reachable second machine).
- Import the issues: `python3 planning/scripts/import_github.py --repo OWNER/NAME --apply` (it creates every milestone's
  issues; the done ones can be closed in bulk).

## Acceptance criteria

- [ ] CI is green on `main`.
- [ ] No `OWNER/shoebox` or `example.invalid` left (`grep -r`).
- [ ] Nightly real-network run green once.

---
_Depends on: R1-01, R1-02, R1-12 · Estimate: 0.5d · Features: —_
