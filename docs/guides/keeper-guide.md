# Keeper guide

A **keeper** is a Shoebox device that is always on and holds every original of your library. It is the
backstop that makes "my phone fell in the sea" a non-event. It can be a Raspberry Pi 4 with a USB disk, a NAS, a
spare computer or a small cloud server.

## Sizing

* **Disk:** your library's original size, plus roughly 10% for thumbnails (256 px) and previews (2,048 px on the long edge) and metadata. Leave 20% free.
  Example: 400 GB of originals → a 1 TB disk.
* **Memory:** the library's metadata lives in memory (about 7 KB per photo per device today), so 100,000
  photos need roughly 700 MB. A Pi 4 with 4 GB is fine.
* **Network:** the first sync copies everything once. On gigabit Ethernet 100 GB takes about 20 minutes; over a
  relay it is limited by your uplink. An interrupted transfer resumes where it stopped.

## Install

Download `shoebox-keeper-<platform>.tar.gz` from the release page (Linux x86-64 and ARM64, macOS), check the
published checksum, and put the binary somewhere on the path:

```bash
tar -xzf shoebox-keeper-linux-aarch64.tar.gz
sudo install shoebox-keeper /usr/local/bin/
```

Or build from source: `cargo build --release -p keeper`.

## Join your library

1. On a device already in the library: *Devices → Add a device → Keeper*. It shows a pairing code.
2. On the keeper machine:

   ```bash
   shoebox-keeper run --name "Pi in the hall" &        # or use the service below
   shoebox-keeper pair <the code> --name "Pi in the hall"
   ```

3. Compare the six digits shown on both screens, answer *yes* on both.

The keeper then syncs the library and downloads every original. `shoebox-keeper status` shows progress
(photos, originals held, at-risk photos, connected devices and how it reaches them).

## Run it as a service

```bash
shoebox-keeper service install            # systemd user unit (Linux) or launchd agent (macOS)
shoebox-keeper service install --system --user shoebox   # system-wide, as a dedicated user (needs root)
shoebox-keeper service install --print    # show the unit without installing
```

Data lives in `~/.local/share/shoebox` (`~/Library/Application Support/shoebox` on macOS) unless you pass
`--storage DIR` or set `$SHOEBOX_STORAGE`. Put it on the big disk. Back up that directory *only* if you also
back up the 24-word recovery phrase separately.

## Everyday commands

| Command | Use |
|---|---|
| `status [--json]` | what the keeper holds and who it talks to |
| `alerts` / `alerts ack` | security alerts: a new admin device, or a replaced recovery phrase; `status` shows them first until you `ack` |
| `devices` | list devices (by id, name, role) |
| `devices remove <device> [--phrase]` | remove a device (a device is its name or the start of its id). The keeper must be an admin, and the device not one; with `--phrase` any device can go |
| `devices role <device> keeper\|companion\|viewer [--phrase]` | change a device's role |
| `devices make-admin <device>` | make a device an admin (asks for your recovery phrase; typing is hidden). Run it on the keeper itself to make the keeper an admin: `devices make-admin <this keeper's name>` |
| `devices replace-phrase` | replace the recovery phrase after it may have been seen; prints the new 24 words once |
| `address` | the link another device needs to recover through this keeper |
| `album list` / `album join <invitation>` / `album keep <id>` / `album drop <id>` | shared albums the keeper is in; `keep` makes it hold every original of an album |
| `stop` | shut the daemon down cleanly |

Full reference: `docs/keeper-cli.md`.

## Upgrading

Stop the service, replace the binary, start it. The data directory carries a format marker: a newer
keeper upgrades older data after copying the small files to `backups/`, and an older keeper refuses data a
newer one wrote (it tells you to update). Devices on neighbouring versions keep syncing with each other.

## Reaching the keeper from outside your home

Nothing needs to be opened on your router: devices find each other through iroh's discovery and, when a direct
path fails, a relay. For a keeper on a cloud server, nothing special is needed either. To avoid the public
relay, see `relay-guide.md`.

## Troubleshooting

* `a keeper is already running on …` – another process holds `keeper.lock`; stop it or use another `--storage`.
* `not paired with a library yet` – run `pair` first.
* Status shows `relay` for a device on the same network – mDNS may be blocked; both devices will still sync.
* Disk full – the keeper keeps what it already has, reports the failure in its log, and resumes by itself when
  space is available.
