# Real-network nightly test (M6-01)

`scripts/real-network/scenario.py` drives the shipped binaries through the story that matters:
the laptop (the app core behind the dev HTTP bridge) creates a library and imports a corpus, a keeper
pairs with it (six-digit code answered on both sides), the keeper syncs the metadata and fetches every
original, then the laptop revokes the keeper and a photo added afterwards must never reach it.

```bash
# self-test on one machine (relay off, direct loopback paths)
cargo build --release -p keeper --bin shoebox-keeper
cargo build --release -p desktop-mobile --features http --bin desktop-dev-server
python3 scripts/real-network/scenario.py --local

# the real thing: keeper on a VPS, this machine behind its NAT, public relays
python3 scripts/real-network/scenario.py --remote user@vps --remote-keeper-bin /usr/local/bin/shoebox-keeper
```

The report (`real-network-report.json`) lists every step with its duration, and how the devices reached each
other (`lan`, `direct` or `relay`, with latency). `.github/workflows/nightly-network.yml` runs it every night
on a GitHub runner against a VPS (secrets `NIGHTLY_VPS_HOST`, `NIGHTLY_VPS_SSH_KEY`), archives the report for
90 days and fails the job on a failed step or a step three times slower than `scripts/real-network/baseline.json`.

## State

* `--local` passes on a laptop (200-photo corpus: pair 4 s, sync 2 s, revoke check 45 s).
* The remote mode and the workflow have **not been run**: there is no VPS or repository remote in this
  project yet. The acceptance criterion ("runs nightly for two weeks with results archived") therefore
  starts when someone provides the VPS and the repository.
* The corpus is the `small` preset (about 185 distinct photos), not 500: raise it with `--preset medium` (10,000)
  or `--corpus DIR`.
