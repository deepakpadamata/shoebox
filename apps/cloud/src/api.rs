//! The HTTP interface. Owner calls (`/v1/...`) are signed by a device of the library (see
//! `shoebox_core::cloud_auth`); public calls (`/s/{share}/...`) need only the share id, which opens nothing
//! without the key in the link's fragment. Errors are JSON: `{"error": "<code>", "message": "..."}`.

use crate::store::{Cid, ShareId, StoreError};
use crate::Service;
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
                "the library is over its storage quota",
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

// ---- owner: blobs ----

async fn put_blob(
    State(s): Svc,
    Path(cid_hex): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let cid: Cid = parse_hex(&cid_hex)?;
    // The body of a blob is its own id, so the signature can be checked before a byte is read.
    let lib = authorize(&s, &headers, &method, &uri, &cid)?;
    if s.store.has_blob(lib, &cid).is_some() {
        return Ok(json_response(StatusCode::OK, json!({ "stored": false })));
    }
    let len = content_length(&headers)?;
    if len > s.config.max_blob {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too-large",
            "that blob is too large",
        ));
    }
    s.store.reserve(lib, len, s.config.library_quota)?;
    let tmp = match receive(&s, body, len, &cid).await {
        Ok(t) => t,
        Err(e) => {
            s.store.release(lib, len);
            return Err(e);
        }
    };
    let dest = s.store.blob_path(lib, &cid);
    if tokio::fs::rename(&tmp, &dest).await.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        s.store.release(lib, len);
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io",
            "could not store",
        ));
    }
    s.store.blob_stored(lib, cid, len);
    Ok(json_response(
        StatusCode::CREATED,
        json!({ "stored": true }),
    ))
}

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
    serve_file(&s.store.blob_path(lib, &cid), &headers, &[], |_| true).await
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
    match s.store.remove_blob(lib, &cid)? {
        Some(path) => {
            let _ = tokio::fs::remove_file(path).await;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        None => Err(ApiError::not_found()),
    }
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

async fn put_object(
    State(s): Svc,
    Path(name): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    if !is_name(&name) {
        return Err(ApiError::bad(
            "an object name is 1 to 80 letters, digits, dots, dashes or underscores",
        ));
    }
    // The client says what the body hashes to (and signs it); the hash is checked as the body arrives.
    let expect: [u8; 32] = headers
        .get("x-content-blake3")
        .and_then(|v| v.to_str().ok())
        .map(parse_hex::<32>)
        .transpose()?
        .ok_or_else(|| ApiError::bad("send X-Content-Blake3"))?;
    let lib = authorize(&s, &headers, &method, &uri, &expect)?;
    let len = content_length(&headers)?;
    if len > MAX_OBJECT {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too-large",
            "that object is too large",
        ));
    }
    s.store.reserve(lib, len, s.config.library_quota)?;
    let tmp = match receive(&s, body, len, &expect).await {
        Ok(t) => t,
        Err(e) => {
            s.store.release(lib, len);
            return Err(e);
        }
    };
    if tokio::fs::rename(&tmp, s.store.object_path(lib, &name))
        .await
        .is_err()
    {
        let _ = tokio::fs::remove_file(&tmp).await;
        s.store.release(lib, len);
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io",
            "could not store",
        ));
    }
    s.store.object_stored(lib, &name, len, len);
    Ok(json_response(StatusCode::CREATED, json!({ "size": len })))
}

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
    match s.store.remove_object(lib, &name) {
        Some(p) => {
            let _ = tokio::fs::remove_file(p).await;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        None => Err(ApiError::not_found()),
    }
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
    s.store.put_share(
        lib,
        id,
        header.manifest_version,
        up.expires_ms,
        up.cids,
        &up.manifest,
        s.config.library_quota,
    )?;
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
    if s.store.delete_share(lib, &id) {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ApiError::not_found())
    }
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
    let (_, version, _, _) = s.store.share_info(&id).ok_or_else(ApiError::not_found)?;
    let key = format!("share:{}", hex::encode(id));
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
    let connect = std::iter::once("'self'".to_string())
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
        .layer(middleware::from_fn(public_cors));
    let owner = Router::new()
        .route("/v1/blobs", get(list_blobs))
        .route("/v1/blobs/has", post(has_blobs))
        .route(
            "/v1/blobs/{cid}",
            put(put_blob).get(get_blob).delete(delete_blob),
        )
        .route("/v1/objects", get(list_objects))
        .route(
            "/v1/objects/{name}",
            put(put_object).get(get_object).delete(delete_object),
        )
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
