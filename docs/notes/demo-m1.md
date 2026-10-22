# M1 demo

`docs/notes/demo-m1.sh` runs the whole M1 story with the `shoebox-dev` CLI, three devices on one machine
(each `--dir` is one device):

```bash
docs/notes/demo-m1.sh
```

| Step | What it shows |
|---|---|
| 1 | `init` and `import`: photos are hashed, EXIF-read, thumbnailed and stored; `ls` lists them with copy counts |
| 2 | `run`: the laptop stays online, serves blobs and metadata, and prints a **ticket** |
| 3 | `join <ticket>` (the Pi, a *keeper*): the library arrives; originals are not there yet |
| 4 | `fetch`: an original comes from the best device that holds it; `holdings` shows two copies; `peers` shows the path and latency |
| 5 | A *viewer* joins, sees everything, fetches only a preview on demand |
| 6 | `status`: how many photos are below the redundancy target |

Commands: `init`, `join`, `run`, `ticket`, `import`, `ls`, `sync`, `peers`, `holdings`, `fetch`, `status`. Add `--json`
for machine-readable output. `--local-only` (used by the script) skips the relay and mDNS; set `DEMO_REAL=1` to use
them, which is how you run it across two real machines: run `init` + `run` on one, copy the printed ticket, run
`join <ticket>` on the other.

**Limits:** one process owns a data directory, so `ls`/`fetch`/... cannot run on a device while its own `run` is up;
use another `--dir` (another "device") to look from the outside. M2 replaces the dev ticket with real pairing and adds
the keeper's control socket.
