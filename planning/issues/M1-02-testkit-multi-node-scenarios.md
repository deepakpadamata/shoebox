---
id: M1-02
title: Testkit: multi-node scenarios
milestone: M1 · Sync core
labels: [area:node, type:test]
estimate_days: 2
depends_on: [M1-01]
features: []
---

# M1-02 · Testkit: multi-node scenarios

## Why

Most sync bugs only appear with three or more nodes and reconnects. A harness makes those tests cheap to write.

## Scope

- `Cluster::new(n)` spins up N in-process nodes with temp dirs, local-only discovery, no relay.
- Helpers: `partition(a,b)`, `heal()`, `take_offline(node)`, `wait_until(pred, timeout)`, fake clock for HLC.
- Assertion helpers: converged libraries, holdings equal to expectations.

## Acceptance criteria

- [ ] An example scenario test (3 nodes, partition, edit, heal, converge) runs in under 10 s in CI.

---
_Depends on: M1-01 · Estimate: 2d · Features: —_
