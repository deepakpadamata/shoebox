---
id: M1-01
title: Node runtime skeleton
milestone: M1 · Sync core
labels: [area:node, type:feature]
estimate_days: 2
depends_on: [M0-06]
features: [F1.1]
---

# M1-01 · Node runtime skeleton

## Why

`shoebox-node` owns the iroh endpoint and every subsystem. The keeper, the app and the tests all go through it.

## Scope

- `Node::open(config)`: data dir layout (`blobs/`, `store/`, `keys/`, `config.toml`), endpoint secret load/generate (file-based for now; platform keystores in M3/M4).
- iroh `Endpoint` + `Router` with registered protocols (blobs, gossip, docs, later `shoebox/pair/0`).
- Graceful shutdown; `tracing` spans per subsystem.
- `Node` facade + `NodeEvent` stream (initially: started, peer connected/disconnected).

## Acceptance criteria

- [ ] Two nodes in one process can open, see each other's `EndpointAddr`, connect, and shut down cleanly.
- [ ] Restarting a node keeps its endpoint id.

---
_Depends on: M0-06 · Estimate: 2d · Features: F1.1_
