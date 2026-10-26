# `shoebox-keeper` CLI and control socket

The keeper is a daemon (`shoebox-keeper run`) plus subcommands that talk to it over a Unix socket
(`<storage>/control.sock`, mode `0600`, newline-delimited JSON). Storage defaults to
`$XDG_DATA_HOME/shoebox` (`~/.local/share/shoebox`, `~/Library/Application Support/shoebox` on macOS) and can
be set with `--storage` or `$SHOEBOX_STORAGE`. `<storage>/keeper.lock` is held while a daemon runs; a second
`run` on the same storage refuses to start.

| Command | What it does |
|---|---|
| `run [--name N] [--relay default\|off\|URL] [--no-mdns]` | run the daemon in the foreground (logs to stderr, journald friendly) |
| `pair <ticket> [--name N] [--yes]` | join a library; shows a 6-digit code to compare with the other device |
| `recover --from <link> [--phrase P] [--name N]` | rejoin a library with the 24-word recovery phrase through a surviving device (prompts for the phrase if omitted) |
| `address` | print this keeper's link (`shoebox-peer1…`), which a recovering device uses to reach it |
| `status [--json]` | library counts, at-risk, devices and paths |
| `devices [--json]` / `devices revoke <id>` | list devices / revoke one (this keeper must be an admin) |
| `album list [--json]` / `album keep <id>` / `album drop <id>` / `album join <invitation>` | shared albums this keeper is in; `keep` fetches every original of the album, `drop` lets them go; ids may be shortened to a unique prefix |
| `service install [--system] [--user U] [--print]` / `service uninstall` | systemd unit (Linux) / launchd plist (macOS) |
| `alerts` / `alerts ack` | security alerts (a new admin, a replaced recovery phrase); `ack` clears them |
| `licences` | this keeper's licence, where its source is, and the third-party notices |
| `stop` | ask the daemon to shut down |

## `status --json` (schema 1)

```json
{
  "schema": 1, "version": "0.1.0", "name": "pi", "device_id": "<64 hex>", "uptime_secs": 123,
  "paired": true, "membership": "member | unknown | removed",
  "library": { "id": "<32 hex>", "photos": 0, "held_originals": 0, "at_risk": 0,
               "redundancy_target": 2, "recovery_kit_saved": false },
  "devices": 3,
  "alerts": [ { "kind": "admin_added | phrase_replaced", "device_id": "<64 hex or empty>", "device_name": "…", "at_ms": 0 } ],
  "peers": [ { "id": "<64 hex>", "name": "Asha's phone", "path": "lan | direct | relay | none", "latency_ms": 12 } ],
  "shared_albums": [ { "id": "<32 hex>", "name": "Trip", "photos": 12, "members": 3, "keep_originals": false } ]
}
```
Unpaired keepers return only the top-level fields with `"paired": false`.

## Control protocol

Request: one JSON object per line, `{"cmd": "status" | "devices" | "revoke" | "pair" | "album" | "shutdown", ...}`.
Reply: `{"type":"ok","data":…}` or `{"type":"error","message":…}`. `pair` is a conversation: the daemon sends
`{"type":"code","sas":"123456","peer_name":…,"peer_role":…}`, the client answers `{"confirm":true|false}`, then the
final reply follows.
