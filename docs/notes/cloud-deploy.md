# Deploying Shoebox Cloud

Shoebox Cloud (`apps/cloud`, [cloud-service.md](cloud-service.md)) runs one of two ways. Both serve the same API and the
same viewer page, and the apps cannot tell them apart: a library points at a URL either way.

| | **Cloudflare** (`apps/cloud-worker`) | **Your own server** (`deploy/selfhost`) |
|---|---|---|
| What runs | A Worker, one Durable Object (the service, its records in the object's SQLite), the viewer as static assets | The `shoebox-cloud` binary behind Caddy, in Docker or under systemd |
| Where the bytes are | R2, through its S3 API | The data folder, or any S3-compatible bucket |
| Machine to look after | None | One small VPS (or a home server with a public address) |
| Cost at rest | R2 storage (10 GB free, then about $0.015/GB-month); no egress fees | The VPS, plus the bucket if you use one |
| Stopping a link | Stops every read at once: the Worker reads the bucket itself, a reader never holds a bucket address | At once without a bucket; with one, an address a page already has works for up to 10 minutes (A-119) |
| Bucket CORS rule | Not needed | Needed with a bucket (below) |
| Backups | Cloudflare keeps the Durable Object's database; R2 holds the bytes | Back up the data folder (and the bucket is the bytes) |
| Limits | One Durable Object holds every record in memory (128 MB): roughly a few hundred thousand blobs in all (A-120) | Memory of the machine: into the low millions of blobs |

Either way the service sees only ciphertext, sizes and times ([threat model](../security/threat-model.md)).

## Cloudflare

You need a Cloudflare account, Node 20+, Rust with the `wasm32-unknown-unknown` target, and what
`scripts/build-viewer.sh` lists (wasm-bindgen-cli 0.2.129; on a Mac, Homebrew's `llvm`). Everything below runs in
`apps/cloud-worker`.

1. **Install and log in.** `npm install`, then `npx wrangler login`.
2. **Make the bucket.** `npx wrangler r2 bucket create shoebox`. Keep it private (no public access, no `r2.dev` URL).
3. **Make an R2 API token** for the service to sign with: in the dashboard, *R2 → Manage API tokens → Create*,
   permission *Object Read & Write*, applied to the `shoebox` bucket only. Note the access key id, the secret and the
   S3 endpoint (`https://<account id>.r2.cloudflarestorage.com`).
4. **Settings.** In `wrangler.toml`, set `S3_ENDPOINT` to that endpoint (and `S3_BUCKET` if you named it otherwise).
   Then the secrets:
   ```bash
   npx wrangler secret put S3_ACCESS_KEY_ID
   npx wrangler secret put S3_SECRET_ACCESS_KEY
   npx wrangler secret put ADMIN_TOKEN        # optional: `openssl rand -hex 32`, for /admin
   ```
   `QUOTA_GIB`, `SHARE_DAILY_GIB`, `CLOSED`, `LIBRARIES` and `CSP_CONNECT` are in `[vars]` with their defaults.
   Registration is open by default: anyone who builds the app can register a library and use its quota. To keep the
   service to your own libraries, deploy with `--var CLOSED:true --var LIBRARIES:<library id>` (hex, space-separated
   for several); a library that registered before stays admitted. Passing them on the command line keeps the ids out
   of the repository, but every deploy must pass them again or the service opens.
5. **Deploy.** `npm run deploy`. It builds the viewer and the Worker (`scripts/build-cloud-worker.sh`; the first build
   installs worker-build into `target/tools`) and uploads both. The service is at
   `https://shoebox.<your subdomain>.workers.dev`.
6. **Your own domain** (optional): uncomment `routes` in `wrangler.toml` with a hostname in a zone on the same account
   and deploy again; Cloudflare makes the DNS record and the certificate.
7. **Point the apps at it**: the library's cloud address in Settings, or `SHOEBOX_TEST_CLOUD_URL` for the check below.

**Checking a deployment.** The sharing test can run against any running service. It makes a library, uploads an album,
opens the link the way the viewer does, asks for a range, reports the link and has the operator reinstate it
(with the token), stops the link and restores the library:

```bash
SHOEBOX_TEST_CLOUD_URL=https://view.example.org SHOEBOX_TEST_ADMIN_TOKEN=… \
  cargo test -p shoebox-cloud --test sharing deployed -- --ignored
```

It leaves a test library behind (a few kilobytes). With `SHOEBOX_TEST_KEEP_LINK=1` it prints the link and leaves it
working, to open in a browser.

**Locally**, `npm run dev` runs the Worker, the Durable Object and the assets in wrangler's simulator on
`http://127.0.0.1:8787`, with the bucket settings and secrets from `.dev.vars` (ignored by git), for example MinIO:

```ini
S3_ENDPOINT=http://localhost:9010
S3_BUCKET=shoebox-worker
S3_REGION=us-east-1
S3_ACCESS_KEY_ID=…
S3_SECRET_ACCESS_KEY=…
ADMIN_TOKEN=local-admin-token
```

`SKIP_VIEWER=1 npm run dev` reuses a viewer already in `apps/web-viewer/dist`.

**How it is put together.** The Worker answers every request. The viewer page comes from the static assets, with the
same CSP and headers the server sends. `/v1/`, `/s/`, `/report/`, `/u/` and `/admin/` go to the Durable Object named
`main`, with the caller's address from `CF-Connecting-IP`. The object runs the service's own router (`shoebox_cloud::router`)
on records in its SQLite database (`SqlRecords`) and on R2 through the S3 API (`FetchHttp`), so owners still upload
straight to R2 by presigned URL and commit. When the service answers a share read with a redirect to R2, the Worker
follows it and streams the bytes back. A Durable Object alarm runs the janitor every 10 minutes.

**Upgrading**: `npm run deploy` again. The records stay in the object's database. A change of record format would need
a migration; there has not been one.

## Your own server

`deploy/selfhost` has a Dockerfile (the service and the viewer page in one small image, run as a non-root user), a
compose file with Caddy in front for HTTPS, an example `.env`, and a systemd unit for running without Docker.

**With Docker**, on a machine whose DNS name points at it, with ports 80 and 443 open:

```bash
git clone https://github.com/deepakpadamata/shoebox && cd shoebox/deploy/selfhost
cp .env.example .env        # set SHOEBOX_DOMAIN; the rest is optional
docker compose up -d --build
```

The first build takes a while (Rust and the viewer's WebAssembly). Caddy gets the certificate. The data is in the
`cloud-data` volume; the image can be built alone with `docker build -f deploy/selfhost/Dockerfile .` from the
repository root.

**Without Docker**: build with `cargo build --release -p shoebox-cloud` and `scripts/build-viewer.sh` (which needs the
`wasm32-unknown-unknown` target and `wasm-bindgen-cli` 0.2.129, see the script's header), then follow the
comments at the top of `deploy/selfhost/shoebox-cloud.service`. Put Caddy (`reverse_proxy 127.0.0.1:8080`) or another
TLS proxy in front, and keep `SHOEBOX_TRUST_PROXY=true` only while the proxy sets `X-Forwarded-For` itself.

**Settings** are flags or environment variables (`shoebox-cloud --help`):

| Variable | Default | |
|---|---|---|
| `SHOEBOX_CLOUD_DATA` | `./cloud-data` (`/data` in the image) | The data folder |
| `SHOEBOX_CLOUD_LISTEN` | `127.0.0.1:8080` (`0.0.0.0:8080` in the image) | |
| `SHOEBOX_CLOUD_VIEWER` | none (`/srv/viewer` in the image) | The viewer page's files |
| `SHOEBOX_QUOTA_GIB`, `SHOEBOX_SHARE_DAILY_GIB` | 2, 1 | Per library; per link per day |
| `SHOEBOX_CLOSED` | off | Refuse libraries that have not registered |
| `SHOEBOX_LIBRARIES` | none | Library ids (hex) admitted even while closed: your own |
| `SHOEBOX_TRUST_PROXY` | off | Take the caller's address from the last `X-Forwarded-For` entry |
| `SHOEBOX_CSP_CONNECT` | none | Extra hosts the page may connect to (relays for Live links), space-separated |
| `SHOEBOX_ADMIN_TOKEN` | none | Turns on `/admin` |
| `SHOEBOX_S3_ENDPOINT`, `_BUCKET`, `_REGION`, `_ACCESS_KEY_ID`, `_SECRET_ACCESS_KEY`, `_VIRTUAL_HOST` | no bucket | Keep the bytes in a bucket |

**With a bucket**, readers' browsers fetch from it by presigned URL, so it needs a CORS rule for the service's origin
(GET and HEAD only; the apps upload, not a browser):

```json
[{ "AllowedOrigins": ["https://view.example.org"], "AllowedMethods": ["GET", "HEAD"],
   "AllowedHeaders": ["range"], "ExposeHeaders": ["content-length", "content-range"], "MaxAgeSeconds": 3600 }]
```

**Back up the data folder.** Without a bucket it holds every byte; with one it holds the records (libraries, shares,
reports, uploads in progress) without which the bucket cannot be served. A file-level copy while the service runs is
consistent enough: every record is written whole by rename, and the per-library `index.log` is rewritten at start.
The proxy must not log paths (a share id is a link's address).

## The operator

With an admin token set, three calls take `Authorization: Bearer <token>`:

```bash
curl -H "authorization: Bearer $TOKEN" https://view.example.org/admin/reports
curl -X POST -H "authorization: Bearer $TOKEN" https://view.example.org/admin/shares/<share id>/reinstate
curl -X DELETE -H "authorization: Bearer $TOKEN" https://view.example.org/admin/shares/<share id>
```

A report switches a link off at once (A-118). The operator sees the share id, the library id, when and the reporter's
words, never the photos (the key is only in the link). Reinstating serves the link again; deleting removes the share
and its manifest. The library keeps its blobs, which the owner can delete. Without a token, `/admin` does not exist.
