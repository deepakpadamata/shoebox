# Assumptions log

Reasonable assumptions made while implementing the issues in `planning/issues/`, recorded for review.
Each entry: **ID** · issue · assumption · why · how to reverse.

## M0

- **A-001 · M0-01** · `shoebox-core` and the other crates use a `0.0.1` workspace version and `publish = false`.
  Why: nothing is released yet. Reverse: bump in `[workspace.package]`.
- **A-002 · M0-02** · `repository` is an `example.invalid` placeholder since no remote exists yet.
  Reverse: set the real URL in `[workspace.package]` once a remote is created.
- **A-003 · M0-02** · Rust edition 2021 and `rust-version = "1.85"` (conservative MSRV); the toolchain itself is `stable`.
  Why: iroh 1.x needs a recent compiler; 2021 avoids edition-2024 churn in macro-heavy deps. Reverse: bump edition/MSRV.
- **A-004 · M0-02** · License files: `LICENSE-APACHE`/`LICENSE-MIT` at the root (copyright "Shoebox contributors",
  text copied from the iroh-docs crate's standard licenses), and `apps/LICENSE-AGPL` (official GNU text).
  `tools/*`, `xtask` and `examples/*` are treated as library-licensed (Apache-2.0 OR MIT), since decision #17
  only names `apps/*` as AGPL. Reverse: change the `license` field in the affected crates.
- **A-005 · M0-02** · `apps/desktop-mobile` is a placeholder `fn main() {}` crate now; the Tauri project replaces it in M3-01.
