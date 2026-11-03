# Shoebox Cloud service (W1-04)

`apps/cloud` (`shoebox-cloud`, AGPL-3.0) is one store for ciphertext that does two jobs: it serves web links to shared
albums and it is the cloud keeper. It never sees a photo, an album name or a key. Design: `planning/epics/W1-web-albums-and-cloud.md`.

```bash
cargo run -p shoebox-cloud -- --data-dir ./cloud-data --listen 127.0.0.1:8080 --viewer-dir ./viewer
```

Flags (each also an environment variable where marked): `--data-dir` (`SHOEBOX_CLOUD_DATA`), `--listen`
(`SHOEBOX_CLOUD_LISTEN`), `--viewer-dir` (`SHOEBOX_CLOUD_VIEWER`), `--quota-gib` (2), `--share-daily-gib` (1),
`--closed` (refuse libraries that have not registered), `--trust-proxy` (client address from the last
`X-Forwarded-For`), `--csp-connect` (a host the viewer may connect to; relays, for Live links).

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
| `PUT /v1/blobs/{cid}` | upload ciphertext; the signature commits to `cid`, so it is checked before the body is read; the bytes must hash to `cid` (422 if not); needs `Content-Length`; 507 over quota |
| `GET /v1/blobs/{cid}` | read it back (ranges) |
| `DELETE /v1/blobs/{cid}` | 409 `in-use` while a share lists it |
| `GET /v1/blobs?after=&limit=` | list, with sizes, 1,000 a page |
| `POST /v1/blobs/has` | JSON list of ids in, the ones held out |
| `PUT/GET/DELETE /v1/objects/{name}`, `GET /v1/objects` | named encrypted objects (library snapshots), up to 512 MiB; `X-Content-Blake3` carries the body hash |
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

## Where things are

```text
<data>/libs/<library>/{lib.postcard, blobs/<cid>, objects/<name>}
<data>/shares/<share_id>/{meta.postcard, manifest}
```

Everything is read into memory at start. That is fine into the low millions of blobs; an object store (S3) behind the same
`Store` calls is the next step if one machine is not enough. The service writes no access log; a deployment's proxy must not
log paths either (a share id in a log is not a key, but it is a link's address).

## Not done here

Billing and per-library quotas (W1-10), the sharer's client (W1-05), the viewer (W1-06).
