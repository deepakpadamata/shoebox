# Shoebox Cloud service (W1-04)

`apps/cloud` (`shoebox-cloud`, AGPL-3.0) is one store for ciphertext that does two jobs: it serves web links to shared
albums and it is the cloud keeper. It never sees a photo, an album name or a key. Design: `planning/epics/W1-web-albums-and-cloud.md`.

```bash
cargo run -p shoebox-cloud -- --data-dir ./cloud-data --listen 127.0.0.1:8080 --viewer-dir ./viewer
```

Flags (each also an environment variable where marked): `--data-dir` (`SHOEBOX_CLOUD_DATA`), `--listen`
(`SHOEBOX_CLOUD_LISTEN`), `--viewer-dir` (`SHOEBOX_CLOUD_VIEWER`), `--quota-gib` (2), `--share-daily-gib` (1),
`--closed` (refuse libraries that have not registered), `--trust-proxy` (client address from the last
`X-Forwarded-For`), `--csp-connect` (a host the viewer may connect to; relays, for Live links). With
`--s3-endpoint` the bytes go to a bucket instead (below).

## Owner calls (signed)

Every `/v1/...` call carries `Authorization: Shoebox <hex>`: a `CloudAuth` (`shoebox_core::cloud_auth`), the sending
device's certificate (and its admin's, if an admin certified it), signed over the method, the path and query, the time
and the BLAKE3 hash of the body, under the domain `shoebox/cloud-request/v1`. A request is good for five minutes either
side of the service's clock. The service keeps the library's root public key from the first request it sees (while
registration is open) and checks every later chain against it: **the first request fixes a library's root, so a library
id must stay as private as the album capability it is.** A revoked device is refused once the service has been sent its
revocation (`POST /v1/library/certs`); until then it still has the old certificate.

| Call | What |
|---|---|
| `POST /v1/uploads` | `{"blobs":[{cid,size}],"objects":[{name,size,hash}]}`, at most 1,000; answers per entry either `stored:true` (already held) or a `token` and a `url` to `PUT` the bytes to; reserves the quota (507 over it). A slot lasts two hours |
| `PUT <url>` | the bytes, with exactly the announced `Content-Length`. Without a bucket the url is `/u/{token}` on the service (unsigned: the token is the permission), which checks the hash at once (422); with a bucket it is a presigned PUT straight to the bucket, which refuses any other length |
| `POST /v1/uploads/commit` | `{"tokens":[...]}`: records what was put; per token `ok` or an error (`not-uploaded`, `unknown-upload` when the slot ran out, `bad-request` for a wrong size). With a bucket the service checks the size with a HEAD and deletes a wrong-sized object. A slot never committed is released and its bytes deleted by the janitor |
| `GET /v1/blobs/{cid}` | read it back (ranges); with a bucket, a 302 to a presigned GET good for 60 s |
| `DELETE /v1/blobs/{cid}` | 409 `in-use` while a share lists it |
| `GET /v1/blobs?after=&limit=` | list, with sizes, 1,000 a page |
| `POST /v1/blobs/has` | JSON list of ids in, the ones held out |
| `GET/DELETE /v1/objects/{name}`, `GET /v1/objects` | named encrypted objects (library snapshots), up to 512 MiB, uploaded through `/v1/uploads` with their BLAKE3 hash |
| `PUT /v1/shares/{share_id}` | a `ShareUpload` (sealed manifest, the ids it may serve, expiry); every id must be uploaded already (400 `missing-blobs`); a lower version is 409 |
| `DELETE /v1/shares/{share_id}`, `GET /v1/shares` | revoke; list |
| `GET /v1/usage` | `{used, quota}` |
| `POST /v1/library/certs` | a `CertBundle`: revocations the service must know |

## Public calls

`GET /s/{share_id}/m` (the manifest) and `GET /s/{share_id}/b/{cid}` (a blob, with ranges). A blob is served only if the share
lists it, so a guessed or leaked id opens nothing; an unknown or deleted share and an unlisted blob are all 404, an expired
share is 410 for a week and then deleted. Per share: 600 requests a minute and 1 GiB a day; per address: 1,200 requests a minute.
Responses carry `Referrer-Policy: no-referrer` and CORS for these two routes. The viewer page is served at `/` and `/a` with a
strict CSP.

## With a bucket (S3-compatible)

```bash
SHOEBOX_S3_ACCESS_KEY_ID=… SHOEBOX_S3_SECRET_ACCESS_KEY=… \
cargo run -p shoebox-cloud -- --data-dir ./cloud-data --viewer-dir ./viewer \
  --s3-endpoint https://<account>.r2.cloudflarestorage.com --s3-bucket shoebox --s3-region auto
```

`--s3-region` defaults to `auto` (R2); `--s3-virtual-host` for providers that want `bucket.host` addressing. The service
then never carries a photo's bytes: owners `PUT` straight to presigned URLs, and every read (`/s/{id}/m`, `/s/{id}/b/{cid}`,
the owner's GETs) answers `302` to a presigned GET (manifest and owner reads `no-store`; a share's blob `private,
max-age=300`). Presigned reads last 10 minutes, writes an hour. The service's own clock signs them, so it must keep good time.
The viewer page's CSP adds the bucket's origin to `connect-src` by itself. Keys in the bucket follow the disk layout:
`libs/<library>/blobs/<cid>`, `libs/<library>/objects/<name>`, `shares/<share_id>/manifest`.

The bucket needs a CORS rule so the viewer can read it, for example (R2/S3 JSON):

```json
[{ "AllowedOrigins": ["https://view.example.org"], "AllowedMethods": ["GET", "HEAD"],
   "AllowedHeaders": ["range"], "ExposeHeaders": ["content-length", "content-range"], "MaxAgeSeconds": 3600 }]
```

Uploads come from the apps, not a browser, so `PUT` needs no CORS rule. The bucket should be private (no public
listing or reads); everything public goes through the service's check that a share lists the blob.

The data directory still holds the bookkeeping: `lib.postcard`, the share records and reports, and per library an
`index.log` of what the bucket holds (`+b <cid> <size>`, `-b <cid>`, `+o <name> <size>`, `-o <name>`, compacted at
start). **Back the data directory up**: without it the bucket is ciphertext nobody can be served (the apps can upload
again; nothing is lost that a device still has). The bucket does not check BLAKE3 hashes, only sizes; a wrong blob is
caught by whoever reads it (the viewer and `cloud_restore` both check the ciphertext against its id) and spoils only the
uploader's own share. One service instance per data directory.

## The operator

With `--admin-token` (`SHOEBOX_ADMIN_TOKEN`), `GET /admin/reports` lists reported shares (share id, library, when, the
reporter's words), `POST /admin/shares/{id}/reinstate` serves one again and `DELETE /admin/shares/{id}` deletes it and
its manifest; each takes `Authorization: Bearer <token>` (compared in constant time). Without a token there is no
`/admin`. How to deploy, on Cloudflare or a server of your own: [cloud-deploy.md](cloud-deploy.md).

## Where things are

```text
<data>/libs/<library>/{lib.postcard, index.log (with a bucket), blobs/<cid>, objects/<name>}
<data>/shares/<share_id>/{meta.postcard, manifest}
<data>/reports/<share_id>
<data>/uploads/<token>        an upload reserved and not yet committed or expired
```

The records go through a small key-value interface (`meta::Meta`): files in the data folder on a server, the Durable
Object's SQLite database on Cloudflare (`apps/cloud-worker`, A-120). An upload in progress is a record too, so a
restart keeps its quota reserved and the janitor still deletes its bytes if it is never committed.

Everything is read into memory at start. That is fine into the low millions of blobs. With a bucket the machine needs
only a small disk for the bookkeeping. The service writes no access log; a deployment's proxy must not
log paths either (a share id in a log is not a key, but it is a link's address).

## Not done here

Billing (W1-10), the sharer's client (W1-05), the viewer (W1-06).
