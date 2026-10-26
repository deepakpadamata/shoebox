---
id: R1-08
title: Repository remote and the first green CI run
milestone: R1 · v0.1 testers' preview
labels: [area:infra, type:chore, needs:owner]
estimate_days: 0.5
depends_on: [R1-01, R1-02]
features: []
---

# R1-08 · Repository remote and the first green CI run

## Why

There is no remote, so CI, the nightly network test and every release workflow have never run (A-006, A-065).

## Scope

- **Owner:** create the repository (private is fine) and give its `OWNER/NAME`.
- Replace the placeholders: `repository` in `[workspace.package]` (A-002), `OWNER/shoebox` in workflows and the install script.
- Push, run CI, fix whatever only fails there (paths, toolchains, the license guard, the TS-bindings diff).
- Optionally import these issues: `python3 planning/scripts/import_github.py --repo OWNER/NAME --apply`.

## Acceptance criteria

- [ ] CI is green on `main`.
- [ ] No `OWNER/shoebox` or `example.invalid` left (`grep -r`).

---
_Depends on: R1-01, R1-02 · Estimate: 0.5d · Features: —_
