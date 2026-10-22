---
id: M6-04
title: Security review and threat model
milestone: M6 · Hardening & beta
labels: [area:security, type:docs]
estimate_days: 3
depends_on: [M5-08]
features: []
---

# M6-04 · Security review and threat model

## Why

Pairing, capabilities, revocation and recovery are security-critical.

## Scope

- Threat model doc: assets, attackers (lost phone, malicious album member, network attacker, relay operator), mitigations.
- Review: ticket entropy and expiry, SAS derivation, cert chain checks, capability handling, keystore usage, log redaction.
- Fix findings; get an external review if budget allows (otherwise a documented self-review).

## Acceptance criteria

- [ ] Threat model committed; every high finding fixed or explicitly accepted with a reason.

---
_Depends on: M5-08 · Estimate: 3d · Features: —_
