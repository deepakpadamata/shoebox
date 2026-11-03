//! The HTTP interface. Owner calls (`/v1/...`) are signed by a device of the library (see
//! `shoebox_core::cloud_auth`); public calls (`/s/{share}/...`) need only the share id, which opens nothing
//! without the key in the link's fragment. Errors are JSON: `{"error": "<code>", "message": "..."}`.
//!
//! Uploads take three steps, so the bytes can go straight to a bucket: `POST /v1/uploads` reserves quota and
//! answers with a URL per blob or object, the client `PUT`s the bytes there, and `POST /v1/uploads/commit` records
//! them. Without a bucket the URL is the service's own `/u/{token}`. With a bucket, reads answer with a redirect
//! to a presigned URL that lasts [`crate::s3::READ_TTL`].

use crate::s3::{self, S3};
use crate::store::{Cid, ShareId, StoreError};
use crate::{Pending, Service, Target};
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::header::{
    ACCEPT_RANGES, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    RANGE,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use shoebox_core::cloud_auth::{CertBundle, CloudAuth};
use shoebox_core::share::{read_header, ShareUpload};
use shoebox_core::store::LibraryId;
use std::net::SocketAddr;
use std::path::Path as FsPath;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;

type Svc = State<Arc<Service>>;

const MAX_SHARE_UPLOAD: usize = 64 * 1024 * 1024;
const MAX_CIDS: usize = 1_000_000;
const MAX_OBJECT: u64 = 512 * 1024 * 1024;
const MAX_HAS: usize = 10_000;
const MAX_UPLOADS: usize = 1_000;
/// An upload not committed by then gives its quota back (a little longer than its URL lasts).
const UPLOAD_LIFE_MS: i64 = 2 * 3600 * 1000;

pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    extra: Option<Value>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            extra: None,
        }
    }
    fn with(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }
    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad-request", message)
    }
    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not-found", "there is nothing here")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.code, "message": self.message });
        if let (Some(Value::Object(extra)), Some(obj)) = (self.extra, body.as_object_mut()) {
            obj.extend(extra);
        }
        json_response(self.status, body)
    }
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::UnknownLibrary | StoreError::WrongRoot => Self::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "this device is not a current member of a library we can serve",
            ),
            StoreError::TooManyCerts => Self::bad("too many certificates for one library"),
            StoreError::QuotaExceeded { used, quota } => Self::new(
                StatusCode::INSUFFICIENT_STORAGE,
                "quota-exceeded",
                "this library is over its storage quota: stop a web link or remove something from the cloud copy, then try again",
            )
            .with(json!({ "used": used, "quota": quota })),
            StoreError::Conflict(why) => Self::new(StatusCode::CONFLICT, "conflict", why),
            StoreError::InUse(shares) => Self::new(
                StatusCode::CONFLICT,
                "in-use",
                "a share still lists this blob",
            )
            .with(json!({ "shares": shares.iter().map(hex::encode).collect::<Vec<_>>() })),
            StoreError::Missing(cids) => Self::bad("the share lists blobs that were not uploaded")
                .with(json!({ "missing": cids.iter().map(hex::encode).collect::<Vec<_>>() })),
        }
    }
}

fn parse_hex<const N: usize>(text: &str) -> Result<[u8; N], ApiError> {
    hex::decode(text)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| ApiError::bad("not a valid id"))
}

fn is_name(name: &str) -> bool {
    (1..=80).contains(&name.len())
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Who is calling? Checks the signature over the request, then that the device is a current member.
fn authorize(
    s: &Service,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body_hash: &[u8; 32],
) -> Result<LibraryId, ApiError> {
    let unauthorised =
        |m: &str| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", m.to_string());
    let text = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Shoebox "))
        .ok_or_else(|| unauthorised("sign the request"))?;
    let auth = CloudAuth::from_header(text).map_err(|e| unauthorised(&e.to_string()))?;
    let target = uri.path_and_query().map_or(uri.path(), |p| p.as_str());
    auth.verify_hashed(
        method.as_str(),
        target,
        body_hash,
        s.now(),
        s.config.skew_ms,
    )
    .map_err(|e| unauthorised(&e.to_string()))?;
    s.store.admit(
        auth.library,
        auth.root,
        auth.admin.as_ref(),
        &auth.device,
        s.config.open_registration,
    )?;
    Ok(auth.library)
}

/// A small body held in memory, with its hash.
async fn small_body(body: Body, limit: usize) -> Result<(bytes::Bytes, [u8; 32]), ApiError> {
    let bytes = axum::body::to_bytes(body, limit).await.map_err(|_| {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too-large",
            "the body is too large",
        )
    })?;
    let hash = *blake3::hash(&bytes).as_bytes();
    Ok((bytes, hash))
}

fn content_length(headers: &HeaderMap) -> Result<u64, ApiError> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::LENGTH_REQUIRED,
                "length-required",
                "send a Content-Length",
            )
        })
}

/// Stream an upload to a temporary file, checking its length and hash. The caller has reserved `declared`.
async fn receive(
    s: &Service,
    body: Body,
    declared: u64,
    expect: &[u8; 32],
) -> Result<std::path::PathBuf, ApiError> {
    let tmp = s.store.tmp_path();
    let result: Result<(), ApiError> = async {
        let mut file = tokio::fs::File::create(&tmp).await.map_err(|_| {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "io", "could not store")
        })?;
        let mut hasher = blake3::Hasher::new();
        let mut total = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ApiError::bad("the upload broke off"))?;
            total += chunk.len() as u64;
            if total > declared {
                return Err(ApiError::bad("more bytes than Content-Length"));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|_| {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "io", "could not store")
            })?;
        }
        file.flush().await.ok();
        if total != declared {
            return Err(ApiError::bad("fewer bytes than Content-Length"));
        }
        if hasher.finalize().as_bytes() != expect {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "hash-mismatch",
                "the bytes do not hash to the id they were sent under",
            ));
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}

// ---- serving a file with ranges ----

fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.checked_sub(n.min(len))?, len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>().ok()?.min(len - 1)
        };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

async fn serve_file(
    path: &FsPath,
    req: &HeaderMap,
    extra: &[(HeaderName, String)],
    charge: impl FnOnce(u64) -> bool,
) -> Result<Response, ApiError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| ApiError::not_found())?;
    let len = file
        .metadata()
        .await
        .map_err(|_| ApiError::not_found())?
        .len();
    let (status, start, end) = match req.get(RANGE).and_then(|v| v.to_str().ok()) {
        Some(r) => match parse_range(r, len) {
            Some((a, b)) => (StatusCode::PARTIAL_CONTENT, a, b),
            None => {
                return Ok((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(CONTENT_RANGE, format!("bytes */{len}"))],
                )
                    .into_response())
            }
        },
        None => (StatusCode::OK, 0, len.saturating_sub(1)),
    };
    let n = if len == 0 { 0 } else { end - start + 1 };
    if !charge(n) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "daily-limit",
            "this link has served all it may today",
        ));
    }
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| ApiError::not_found())?;
    let mut res = Response::new(Body::from_stream(ReaderStream::with_capacity(
        file.take(n),
        64 * 1024,
    )));
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(CONTENT_LENGTH, HeaderValue::from(n));
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{len}")) {
            h.insert(CONTENT_RANGE, v);
        }
    }
    for (name, value) in extra {
        if let Ok(v) = HeaderValue::from_str(value) {
            h.insert(name.clone(), v);
        }
    }
    Ok(res)
}

// ---- owner: uploads ----

#[derive(Deserialize)]
struct BlobWanted {
    cid: String,
    size: u64,
}

#[derive(Deserialize)]
struct ObjectWanted {
    name: String,
    size: u64,
    hash: String,
}

#[derive(Deserialize)]
struct UploadRequest {
    #[serde(default)]
    blobs: Vec<BlobWanted>,
    #[serde(default)]
    objects: Vec<ObjectWanted>,
}

fn new_token() -> [u8; 16] {
    let mut t = [0u8; 16];
    getrandom::fill(&mut t).expect("operating system randomness");
    t
}

fn uploads(s: &Service) -> std::sync::MutexGuard<'_, std::collections::HashMap<[u8; 16], Pending>> {
    s.uploads.lock().unwrap_or_else(|e| e.into_inner())
}

/// `POST /v1/uploads`: reserve quota for blobs and objects and say where to put each. A blob already held answers
/// `stored`; nothing is reserved for it.
async fn begin_uploads(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let (bytes, hash) = small_body(body, 1024 * 1024).await?;
    let lib = authorize(&s, &headers, &method, &uri, &hash)?;
    let req: UploadRequest =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("not an upload request"))?;
    if req.blobs.len() + req.objects.len() > MAX_UPLOADS {
        return Err(ApiError::bad("too many uploads in one request"));
    }
    let mut wanted: Vec<(Target, u64, [u8; 32])> = vec![];
    for b in &req.blobs {
        let cid: Cid = parse_hex(&b.cid)?;
        if b.size > s.config.max_blob {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too-large",
                "that blob is too large",
            ));
        }
        wanted.push((Target::Blob(cid), b.size, cid));
    }
    for o in &req.objects {
        if !is_name(&o.name) {
            return Err(ApiError::bad(
                "an object name is 1 to 80 letters, digits, dots, dashes or underscores",
            ));
        }
        if o.size > MAX_OBJECT {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too-large",
                "that object is too large",
            ));
        }
        wanted.push((Target::Object(o.name.clone()), o.size, parse_hex(&o.hash)?));
    }

    let now = s.now();
    let mut reserved: Vec<[u8; 16]> = vec![];
    let mut out = vec![];
    for (target, size, hash) in wanted {
        let label = match &target {
            Target::Blob(cid) => json!({ "cid": hex::encode(cid) }),
            Target::Object(name) => json!({ "name": name }),
        };
        if let Target::Blob(cid) = &target {
            if s.store.has_blob(lib, cid).is_some() {
                out.push(merge(label, json!({ "stored": true })));
                continue;
            }
        }
        if let Err(e) = s.store.reserve(lib, size, s.config.library_quota) {
            // Give back what this request reserved: the client gets nothing to upload.
            let mut u = uploads(&s);
            for t in &reserved {
                if let Some(p) = u.remove(t) {
                    s.store.release(p.lib, p.size);
                }
            }
            return Err(e.into());
        }
        let token = new_token();
        let url = match &s.s3 {
            Some(b) => {
                let key = match &target {
                    Target::Blob(cid) => s3::blob_key(&lib.0, cid),
                    Target::Object(name) => s3::object_key(&lib.0, name),
                };
                b.put_url(&key, size, now).to_string()
            }
            None => format!("/u/{}", hex::encode(token)),
        };
        uploads(&s).insert(
            token,
            Pending {
                lib,
                target,
                size,
                hash,
                expires_ms: now + UPLOAD_LIFE_MS,
                done: false,
            },
        );
        reserved.push(token);
        out.push(merge(
            label,
            json!({ "token": hex::encode(token), "url": url }),
        ));
    }
    Ok(json_response(StatusCode::OK, json!({ "uploads": out })))
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
    a
}

/// `PUT /u/{token}`: the bytes of an upload, when they go through the service (no bucket). The token is the
/// permission: it was handed to a signed request, is random, and lasts two hours.
async fn put_upload(
    State(s): Svc,
    Path(token_hex): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    if s.s3.is_some() {
        return Err(ApiError::not_found());
    }
    let token: [u8; 16] = parse_hex(&token_hex).map_err(|_| ApiError::not_found())?;
    let (lib, target, size, hash) = {
        let u = uploads(&s);
        let p = u
            .get(&token)
            .filter(|p| !p.done && p.expires_ms > s.now())
            .ok_or_else(ApiError::not_found)?;
        (p.lib, p.target.clone(), p.size, p.hash)
    };
    // A failed upload ends it and gives the quota back; the client asks again.
    let give_up = |e: ApiError| {
        if uploads(&s).remove(&token).is_some() {
            s.store.release(lib, size);
        }
        e
    };
    if content_length(&headers)? != size {
        return Err(give_up(ApiError::bad(
            "send the size the upload was asked for",
        )));
    }
    let tmp = receive(&s, body, size, &hash).await.map_err(give_up)?;
    let dest = match &target {
        Target::Blob(cid) => s.store.blob_path(lib, cid),
        Target::Object(name) => s.store.object_path(lib, name),
    };
    if tokio::fs::rename(&tmp, &dest).await.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(give_up(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io",
            "could not store",
        )));
    }
    match &target {
        Target::Blob(cid) => s.store.blob_stored(lib, *cid, size),
        Target::Object(name) => s.store.object_stored(lib, name, size, size),
    }
    if let Some(p) = uploads(&s).get_mut(&token) {
        p.done = true;
    }
    Ok(json_response(
        StatusCode::CREATED,
        json!({ "stored": true }),
    ))
}

#[derive(Deserialize)]
struct CommitRequest {
    tokens: Vec<String>,
}

/// `POST /v1/uploads/commit`: record uploads whose bytes are in place. With a bucket, the service checks the size
/// there; a different size is deleted. Each token gets `ok` or an error of its own.
async fn commit_uploads(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let (bytes, hash) = small_body(body, 256 * 1024).await?;
    let lib = authorize(&s, &headers, &method, &uri, &hash)?;
    let req: CommitRequest =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("not a commit request"))?;
    if req.tokens.len() > MAX_UPLOADS {
        return Err(ApiError::bad("too many uploads in one request"));
    }
    let mut results = vec![];
    for t in &req.tokens {
        let r = commit_one(&s, lib, t).await;
        results.push(match r {
            Ok(()) => json!({ "token": t, "ok": true }),
            Err(e) => json!({ "token": t, "error": e.code, "message": e.message }),
        });
    }
    Ok(json_response(StatusCode::OK, json!({ "results": results })))
}

async fn commit_one(
    s: &Service,
    lib: shoebox_core::store::LibraryId,
    token_hex: &str,
) -> Result<(), ApiError> {
    let token: [u8; 16] = parse_hex(token_hex)?;
    let (target, size, done) = {
        let u = uploads(s);
        let p = u.get(&token).filter(|p| p.lib == lib).ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "unknown-upload",
                "no such upload, or it ran out",
            )
        })?;
        (p.target.clone(), p.size, p.done)
    };
    let Some(b) = &s.s3 else {
        if !done {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "not-uploaded",
                "the bytes have not arrived",
            ));
        }
        uploads(s).remove(&token);
        return Ok(());
    };
    let key = match &target {
        Target::Blob(cid) => s3::blob_key(&lib.0, cid),
        Target::Object(name) => s3::object_key(&lib.0, name),
    };
    let found = b.head(&key, s.now()).await.map_err(bucket_error)?;
    match found {
        None => Err(ApiError::new(
            StatusCode::CONFLICT,
            "not-uploaded",
            "the bytes have not arrived",
        )),
        Some(n) if n != size => {
            let _ = b.delete(&key, s.now()).await;
            if uploads(s).remove(&token).is_some() {
                s.store.release(lib, size);
            }
            Err(ApiError::bad(
                "the bucket holds a different size than was asked for",
            ))
        }
        Some(_) => {
            // Only the request that takes the pending upload out records it (two commits of one token count once).
            if uploads(s).remove(&token).is_some() {
                match &target {
                    Target::Blob(cid) => s.store.blob_stored(lib, *cid, size),
                    Target::Object(name) => s.store.object_stored(lib, name, size, size),
                }
            }
            Ok(())
        }
    }
}

fn bucket_error(e: anyhow::Error) -> ApiError {
    tracing::error!("bucket: {e:#}");
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "storage",
        "the storage behind the service did not answer; try again",
    )
}

/// A redirect to a presigned URL in the bucket.
fn redirect(url: &url::Url, cache: &'static str) -> Response {
    let mut res = StatusCode::FOUND.into_response();
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(url.as_str()) {
        h.insert(axum::http::header::LOCATION, v);
    }
    h.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

// ---- owner: blobs ----

async fn get_blob(
    State(s): Svc,
    Path(cid_hex): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let cid: Cid = parse_hex(&cid_hex)?;
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    s.store
        .has_blob(lib, &cid)
        .ok_or_else(ApiError::not_found)?;
    if let Some(b) = &s.s3 {
        return Ok(redirect(
            &b.get_url(&s3::blob_key(&lib.0, &cid), s.now()),
            "no-store",
        ));
    }
    serve_file(&s.store.blob_path(lib, &cid), &headers, &[], |_| true).await
}

/// Delete the bytes of a blob or object the store has just forgotten.
async fn delete_bytes(s: &Service, path: std::path::PathBuf, key: String) -> Result<(), ApiError> {
    match &s.s3 {
        Some(b) => b.delete(&key, s.now()).await.map_err(bucket_error),
        None => {
            let _ = tokio::fs::remove_file(path).await;
            Ok(())
        }
    }
}

async fn delete_blob(
    State(s): Svc,
    Path(cid_hex): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let cid: Cid = parse_hex(&cid_hex)?;
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    if !s.store.remove_blob(lib, &cid)? {
        return Err(ApiError::not_found());
    }
    delete_bytes(&s, s.store.blob_path(lib, &cid), s3::blob_key(&lib.0, &cid)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn has_blobs(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let (bytes, hash) = small_body(body, 1024 * 1024).await?;
    let lib = authorize(&s, &headers, &method, &uri, &hash)?;
    let asked: Vec<String> =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("send a JSON list of ids"))?;
    if asked.len() > MAX_HAS {
        return Err(ApiError::bad("too many ids in one question"));
    }
    let mut present = vec![];
    for a in &asked {
        let cid: Cid = parse_hex(a)?;
        if s.store.has_blob(lib, &cid).is_some() {
            present.push(a.clone());
        }
    }
    Ok(json_response(StatusCode::OK, json!({ "present": present })))
}

#[derive(Deserialize)]
struct ListQuery {
    after: Option<String>,
    limit: Option<usize>,
}

async fn list_blobs(
    State(s): Svc,
    Query(q): Query<ListQuery>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    let after = q.after.as_deref().map(parse_hex::<32>).transpose()?;
    let limit = q.limit.unwrap_or(1000).clamp(1, 5000);
    let page = s.store.list_blobs(lib, after, limit);
    let next = (page.len() == limit)
        .then(|| page.last().map(|(c, _)| hex::encode(c)))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        json!({
            "blobs": page.iter().map(|(c, n)| json!({ "cid": hex::encode(c), "size": n })).collect::<Vec<_>>(),
            "next": next,
        }),
    ))
}

// ---- owner: objects (encrypted library state) ----

async fn get_object(
    State(s): Svc,
    Path(name): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    if !is_name(&name) || s.store.object_len(lib, &name).is_none() {
        return Err(ApiError::not_found());
    }
    if let Some(b) = &s.s3 {
        return Ok(redirect(
            &b.get_url(&s3::object_key(&lib.0, &name), s.now()),
            "no-store",
        ));
    }
    serve_file(&s.store.object_path(lib, &name), &headers, &[], |_| true).await
}

async fn delete_object(
    State(s): Svc,
    Path(name): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    if !is_name(&name) {
        return Err(ApiError::not_found());
    }
    if !s.store.remove_object(lib, &name) {
        return Err(ApiError::not_found());
    }
    delete_bytes(
        &s,
        s.store.object_path(lib, &name),
        s3::object_key(&lib.0, &name),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_objects(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    Ok(json_response(
        StatusCode::OK,
        json!({ "objects": s.store.list_objects(lib).iter().map(|(n, z)| json!({ "name": n, "size": z })).collect::<Vec<_>>() }),
    ))
}

// ---- owner: shares, usage, certificates ----

async fn put_share(
    State(s): Svc,
    Path(id_hex): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let id: ShareId = parse_hex(&id_hex)?;
    let (bytes, hash) = small_body(body, MAX_SHARE_UPLOAD).await?;
    let lib = authorize(&s, &headers, &method, &uri, &hash)?;
    let up: ShareUpload =
        postcard::from_bytes(&bytes).map_err(|_| ApiError::bad("not a share upload"))?;
    if up.cids.len() > MAX_CIDS {
        return Err(ApiError::bad("too many blobs in one share"));
    }
    let header = read_header(&up.manifest).map_err(|e| ApiError::bad(e.to_string()))?;
    if header.share_id != id {
        return Err(ApiError::bad("the manifest is for another share id"));
    }
    let len = up.manifest.len() as u64;
    let quota = s.config.library_quota;
    let version = header.manifest_version;
    match &s.s3 {
        Some(b) => {
            // Checked before the manifest goes to the bucket, so a refused share never replaces a served one.
            s.store
                .check_share(lib, &id, version, &up.cids, len, quota)?;
            b.put(&s3::manifest_key(&id), up.manifest, s.now())
                .await
                .map_err(bucket_error)?;
            s.store
                .put_share(lib, id, version, up.expires_ms, up.cids, None, len, quota)?;
        }
        None => s.store.put_share(
            lib,
            id,
            version,
            up.expires_ms,
            up.cids,
            Some(&up.manifest),
            len,
            quota,
        )?,
    }
    Ok(json_response(
        StatusCode::OK,
        json!({ "version": header.manifest_version }),
    ))
}

async fn delete_share(
    State(s): Svc,
    Path(id_hex): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let id: ShareId = parse_hex(&id_hex)?;
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    if !s.store.delete_share(lib, &id) {
        return Err(ApiError::not_found());
    }
    if let Some(b) = &s.s3 {
        b.delete(&s3::manifest_key(&id), s.now())
            .await
            .map_err(bucket_error)?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_shares(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    Ok(json_response(
        StatusCode::OK,
        json!({ "shares": s.store.shares_of(lib).iter().map(|(id, v, e, n)| json!({
            "shareId": hex::encode(id), "version": v, "expiresMs": e, "manifestBytes": n,
            "reported": s.store.report_of(id).map(|(at, why)| json!({ "atMs": at, "reason": why })),
        })).collect::<Vec<_>>() }),
    ))
}

async fn usage(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lib = authorize(&s, &headers, &method, &uri, blake3::hash(b"").as_bytes())?;
    Ok(json_response(
        StatusCode::OK,
        json!({ "used": s.store.usage(lib), "quota": s.config.library_quota }),
    ))
}

async fn post_certs(
    State(s): Svc,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let (bytes, hash) = small_body(body, 1024 * 1024).await?;
    let lib = authorize(&s, &headers, &method, &uri, &hash)?;
    let bundle: CertBundle =
        postcard::from_bytes(&bytes).map_err(|_| ApiError::bad("not a certificate bundle"))?;
    s.store.add_certs(lib, bundle.admins, bundle.devices)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- public: a share's manifest and blobs ----

fn public_share(s: &Service, id: &ShareId) -> Result<(), ApiError> {
    let (_, _, expires, _) = s.store.share_info(id).ok_or_else(ApiError::not_found)?;
    if s.store.report_of(id).is_some() {
        return Err(ApiError::new(
            StatusCode::GONE,
            "reported",
            "this link was reported and is switched off while it is looked at",
        ));
    }
    if expires.is_some_and(|e| s.now() >= e) {
        return Err(ApiError::new(
            StatusCode::GONE,
            "expired",
            "this link has expired",
        ));
    }
    if !s.limiter.hit(
        &format!("share:{}", hex::encode(id)),
        s.now(),
        s.config.share_requests_per_min,
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate-limited",
            "slow down",
        ));
    }
    Ok(())
}

async fn get_manifest(
    State(s): Svc,
    Path(id_hex): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let id: ShareId = parse_hex(&id_hex).map_err(|_| ApiError::not_found())?;
    public_share(&s, &id)?;
    let (_, version, _, len) = s.store.share_info(&id).ok_or_else(ApiError::not_found)?;
    let key = format!("share:{}", hex::encode(id));
    if let Some(b) = &s.s3 {
        charge_or_refuse(&s, &key, len)?;
        return Ok(redirect(
            &b.get_url(&s3::manifest_key(&id), s.now()),
            "no-store",
        ));
    }
    serve_file(
        &s.store.manifest_path(&id),
        &headers,
        &[
            (CACHE_CONTROL, "no-cache".into()),
            (ETAG, format!("\"{version}\"")),
        ],
        |n| {
            s.limiter
                .charge(&key, s.now(), n, s.config.share_daily_bytes)
        },
    )
    .await
}

async fn get_share_blob(
    State(s): Svc,
    Path((id_hex, cid_hex)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let id: ShareId = parse_hex(&id_hex).map_err(|_| ApiError::not_found())?;
    let cid: Cid = parse_hex(&cid_hex).map_err(|_| ApiError::not_found())?;
    public_share(&s, &id)?;
    // Only a blob the share lists, and only from the library that owns the share.
    let lib = s
        .store
        .share_serves(&id, &cid)
        .ok_or_else(ApiError::not_found)?;
    let key = format!("share:{}", hex::encode(id));
    if let Some(b) = &s.s3 {
        // The whole blob counts against the day, whatever part of it the reader then fetches.
        let len = s
            .store
            .has_blob(lib, &cid)
            .ok_or_else(ApiError::not_found)?;
        charge_or_refuse(&s, &key, len)?;
        return Ok(redirect(
            &b.get_url(&s3::blob_key(&lib.0, &cid), s.now()),
            "private, max-age=300",
        ));
    }
    serve_file(
        &s.store.blob_path(lib, &cid),
        &headers,
        &[(CACHE_CONTROL, "private, max-age=600".into())],
        |n| {
            s.limiter
                .charge(&key, s.now(), n, s.config.share_daily_bytes)
        },
    )
    .await
}

fn charge_or_refuse(s: &Service, key: &str, n: u64) -> Result<(), ApiError> {
    if s.limiter
        .charge(key, s.now(), n, s.config.share_daily_bytes)
    {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "daily-limit",
            "this link has served all it may today",
        ))
    }
}

/// `POST /report/{share}`: anyone who has a link can switch it off. The service knows nothing about what is in it,
/// so it takes the share's word-free id and a few words of why. A share is taken down at once and the owner sees
/// the reason in the app; the operator reviews and either deletes the share or deletes `reports/<id>` to reinstate it.
async fn report(
    State(s): Svc,
    Path(id_hex): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let id: ShareId = parse_hex(&id_hex).map_err(|_| ApiError::not_found())?;
    // Few reports per share and per service a minute: a person reporting is rare, a script is not.
    if !s.limiter.hit(&format!("report:{id_hex}"), s.now(), 5)
        || !s.limiter.hit("report:all", s.now(), 120)
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate-limited",
            "slow down",
        ));
    }
    let (bytes, _) = small_body(body, 2048).await?;
    let reason: String = String::from_utf8_lossy(&bytes)
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    if !s.store.report_share(&id, s.now(), reason.trim()) {
        return Err(ApiError::not_found());
    }
    Ok(json_response(
        StatusCode::ACCEPTED,
        json!({ "reported": true }),
    ))
}

async fn preflight() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

// ---- the viewer page ----

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn csp(s: &Service) -> String {
    // With a bucket, reads are redirected there, and `connect-src` applies to where a redirect leads.
    let connect = std::iter::once("'self'".to_string())
        .chain(s.s3.as_ref().map(S3::origin))
        .chain(s.config.csp_connect.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' blob: data:; \
         media-src blob:; connect-src {connect}; worker-src 'self' blob:; base-uri 'none'; \
         frame-ancestors 'none'; form-action 'none'"
    )
}

async fn viewer(State(s): Svc, uri: Uri) -> Result<Response, ApiError> {
    let dir = s
        .config
        .viewer_dir
        .as_ref()
        .ok_or_else(ApiError::not_found)?;
    let path = uri.path();
    let rel = if path == "/" || path == "/a" {
        "index.html".to_string()
    } else {
        path.trim_start_matches('/').to_string()
    };
    if rel
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == ".." || p.contains('\\'))
    {
        return Err(ApiError::not_found());
    }
    let bytes = tokio::fs::read(dir.join(&rel))
        .await
        .map_err(|_| ApiError::not_found())?;
    let mut res = Response::new(Body::from(bytes));
    let h = res.headers_mut();
    let ct = content_type(&rel);
    h.insert(CONTENT_TYPE, HeaderValue::from_static(ct));
    if ct.starts_with("text/html") {
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        if let Ok(v) = HeaderValue::from_str(&csp(&s)) {
            h.insert("content-security-policy", v);
        }
    } else {
        h.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600"),
        );
    }
    Ok(res)
}

// ---- layers ----

async fn client_ip(State(s): Svc, req: Request, next: Next) -> Response {
    let forwarded = s
        .config
        .trust_proxy
        .then(|| {
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .map(|v| v.trim().to_string())
        })
        .flatten();
    let ip = forwarded
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string())
        })
        .unwrap_or_else(|| "local".to_string());
    if !s
        .limiter
        .hit(&format!("ip:{ip}"), s.now(), s.config.ip_requests_per_min)
    {
        return ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate-limited", "slow down")
            .into_response();
    }
    next.run(req).await
}

async fn hardening(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    res
}

async fn public_cors(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    h.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("range"),
    );
    h.insert(
        "access-control-expose-headers",
        HeaderValue::from_static("content-range, content-length, etag"),
    );
    res
}

pub(crate) fn router(service: Arc<Service>) -> Router {
    let public = Router::new()
        .route("/s/{share}/m", get(get_manifest).options(preflight))
        .route("/s/{share}/b/{cid}", get(get_share_blob).options(preflight))
        .route("/report/{share}", post(report).options(preflight))
        .route("/u/{token}", put(put_upload))
        .layer(middleware::from_fn(public_cors));
    let owner = Router::new()
        .route("/v1/blobs", get(list_blobs))
        .route("/v1/blobs/has", post(has_blobs))
        .route("/v1/blobs/{cid}", get(get_blob).delete(delete_blob))
        .route("/v1/uploads", post(begin_uploads))
        .route("/v1/uploads/commit", post(commit_uploads))
        .route("/v1/objects", get(list_objects))
        .route("/v1/objects/{name}", get(get_object).delete(delete_object))
        .route("/v1/shares", get(list_shares))
        .route("/v1/shares/{share}", put(put_share).delete(delete_share))
        .route("/v1/usage", get(usage))
        .route("/v1/library/certs", post(post_certs));
    Router::new()
        .merge(public)
        .merge(owner)
        .fallback(get(viewer))
        .layer(middleware::from_fn_with_state(service.clone(), client_ip))
        .layer(middleware::from_fn(hardening))
        .with_state(service)
}
