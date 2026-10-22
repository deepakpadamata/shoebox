---
id: M1-07
title: Developer CLI harness
milestone: M1 · Sync core
labels: [area:infra, type:chore]
estimate_days: 1
depends_on: [M1-06]
features: []
---

# M1-07 · Developer CLI harness

## Why

M1 and M2 have no UI. A small CLI makes it possible to demo and debug on real machines.

## Scope

- `tools/shoebox-dev` (Apache): `init`, `import <path>`, `ls`, `ticket`, `join <ticket>`, `peers`, `holdings <id>`, `fetch <id>`.
- Human-readable output and `--json` output.

## Acceptance criteria

- [ ] An M1 demo script (`docs/notes/demo-m1.md`, written as part of this issue) runs end-to-end with this CLI.

---
_Depends on: M1-06 · Estimate: 1d · Features: —_
