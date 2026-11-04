//! Shoebox Cloud on Cloudflare (`docs/notes/cloud-deploy.md`).
//!
//! - **The Worker** (this file's `fetch`) serves the viewer page from the static assets with its security headers,
//!   and hands every API call to the Durable Object. A share's manifest and blobs it fetches from R2 itself and
//!   streams back, so a reader never holds a bucket URL: stopping a link stops it at once.
//! - **The Durable Object** ([`Cloud`]) is the service of `apps/cloud`, run unchanged: one instance for the whole
//!   deployment, its records in the object's SQLite database ([`records::SqlRecords`]) and the bytes in R2 through
//!   the S3 API ([`fetch::FetchHttp`]), so uploads still go straight to the bucket by presigned URL.
#![cfg(target_arch = "wasm32")]

mod fetch;
mod records;

use shoebox_cloud::s3::S3Config;
use shoebox_cloud::{Config, Service};
use std::cell::OnceCell;
use std::sync::Arc;
use tower::ServiceExt;
use worker::{
    durable_object, event, Context, Date, DurableObject, Env, Fetch, Headers, Method, Request,
    RequestInit, RequestRedirect, Response, Result, State,
};

/// Paths the service answers; everything else is the viewer page.
const SERVICE_PATHS: [&str; 5] = ["/v1/", "/s/", "/report/", "/u/", "/admin/"];
/// How often the janitor runs (expired shares, uploads never finished).
const SWEEP_EVERY_MS: i64 = 10 * 60 * 1000;
const GIB: u64 = 1024 * 1024 * 1024;

fn var(env: &Env, name: &str) -> Option<String> {
    // An empty variable (as `wrangler.toml` ships them) does not hide a secret of the same name.
    let set = |v: String| (!v.trim().is_empty()).then_some(v);
    env.var(name)
        .ok()
        .and_then(|v| set(v.to_string()))
        .or_else(|| env.secret(name).ok().and_then(|v| set(v.to_string())))
}

/// The service's settings, from the Worker's variables and secrets (`wrangler.toml`, `wrangler secret put`).
fn config(env: &Env) -> Result<Config> {
    let mut c = Config::new(Default::default());
    c.clock = Arc::new(|| Date::now().as_millis() as i64);
    if let Some(n) = var(env, "QUOTA_GIB").and_then(|v| v.parse::<u64>().ok()) {
        c.library_quota = n * GIB;
    }
    if let Some(n) = var(env, "SHARE_DAILY_GIB").and_then(|v| v.parse::<u64>().ok()) {
        c.share_daily_bytes = n * GIB;
    }
    c.open_registration = var(env, "CLOSED").is_none_or(|v| v != "true");
    // The Worker in front puts the caller's address here (from `CF-Connecting-IP`), replacing whatever was sent.
    c.trust_proxy = true;
    c.csp_connect = var(env, "CSP_CONNECT")
        .map(|v| v.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    c.admin_token = var(env, "ADMIN_TOKEN");
    let need = |name: &str| {
        var(env, name).ok_or_else(|| worker::Error::RustError(format!("{name} is not set")))
    };
    let endpoint = need("S3_ENDPOINT")?;
    c.s3 = Some(S3Config {
        endpoint: endpoint
            .parse()
            .map_err(|_| worker::Error::RustError("S3_ENDPOINT is not a URL".into()))?,
        bucket: need("S3_BUCKET")?,
        region: var(env, "S3_REGION").unwrap_or_else(|| "auto".into()),
        access_key: need("S3_ACCESS_KEY_ID")?,
        secret_key: need("S3_SECRET_ACCESS_KEY")?,
        path_style: var(env, "S3_VIRTUAL_HOST").is_none_or(|v| v != "true"),
    });
    Ok(c)
}

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let path = req.path();
    if SERVICE_PATHS.iter().any(|p| path.starts_with(p)) {
        return to_service(req, &env, &path).await;
    }
    page(&req, &env, &path).await
}

/// Hand a call to the Durable Object, with the caller's address, and stream a share's bytes from the bucket.
async fn to_service(req: Request, env: &Env, path: &str) -> Result<Response> {
    let headers = Headers::new();
    for (name, value) in req.headers().entries() {
        if name != "x-forwarded-for" {
            headers.append(&name, &value)?;
        }
    }
    let ip = req.headers().get("cf-connecting-ip")?.unwrap_or_default();
    headers.set("x-forwarded-for", &ip)?;
    let range = req.headers().get("range")?;
    let method = req.method();
    let mut init = RequestInit::new();
    // The service's 302 to the bucket is for this Worker to follow (or the client, for an upload's address).
    init.with_method(method.clone())
        .with_headers(headers)
        .with_redirect(RequestRedirect::Manual);
    if !matches!(method, Method::Get | Method::Head) {
        init.with_body(req.inner().body().map(Into::into));
    }
    let forwarded = Request::new_with_init(req.url()?.as_str(), &init)?;
    let stub = env
        .durable_object("CLOUD")?
        .id_from_name("main")?
        .get_stub()?;
    let res = stub.fetch_with_request(forwarded).await?;
    if path.starts_with("/s/") && res.status_code() == 302 {
        return from_bucket(res, range).await;
    }
    Ok(res)
}

/// Follow the service's redirect to the bucket here, so the reader gets the bytes and never the presigned URL.
async fn from_bucket(redirect: Response, range: Option<String>) -> Result<Response> {
    let Some(location) = redirect.headers().get("location")? else {
        return Ok(redirect);
    };
    let ask = Headers::new();
    if let Some(r) = &range {
        ask.set("range", r)?;
    }
    let mut init = RequestInit::new();
    init.with_headers(ask);
    let got = Fetch::Request(Request::new_with_init(&location, &init)?)
        .send()
        .await?;
    let status = got.status_code();
    if !(200..300).contains(&status) && status != 416 {
        let body = format!(
            r#"{{"error":"storage","message":"the storage behind the service answered {status}; try again"}}"#
        );
        let h = Headers::new();
        h.set("content-type", "application/json")?;
        h.set("access-control-allow-origin", "*")?;
        return Ok(Response::ok(body)?.with_status(502).with_headers(h));
    }
    let out = Headers::new();
    for name in ["content-length", "content-range", "accept-ranges", "etag"] {
        if let Some(v) = got.headers().get(name)? {
            out.set(name, &v)?;
        }
    }
    out.set("content-type", "application/octet-stream")?;
    for name in [
        "cache-control",
        "access-control-allow-origin",
        "access-control-allow-headers",
        "access-control-expose-headers",
        "referrer-policy",
        "x-content-type-options",
    ] {
        if let Some(v) = redirect.headers().get(name)? {
            out.set(name, &v)?;
        }
    }
    Ok(got.with_headers(out))
}

/// The viewer page from the static assets, with the headers the server sends (strict CSP, no referrer).
async fn page(req: &Request, env: &Env, path: &str) -> Result<Response> {
    let asset = if path == "/" || path == "/a" {
        "/index.html"
    } else {
        path
    };
    let mut url = req.url()?;
    url.set_path(asset);
    url.set_query(None);
    let res = env.assets("ASSETS")?.fetch(url.to_string(), None).await?;
    let h = Headers::new();
    h.set("referrer-policy", "no-referrer")?;
    h.set("x-content-type-options", "nosniff")?;
    if res.status_code() == 404 {
        h.set("content-type", "application/json")?;
        return Ok(
            Response::ok(r#"{"error":"not-found","message":"there is nothing here"}"#)?
                .with_status(404)
                .with_headers(h),
        );
    }
    let content_type = res
        .headers()
        .get("content-type")?
        .unwrap_or_else(|| "application/octet-stream".into());
    if content_type.starts_with("text/html") {
        h.set("cache-control", "no-cache")?;
        h.set(
            "content-security-policy",
            &shoebox_cloud::page_csp(&config_for_page(env), None),
        )?;
    } else {
        h.set("cache-control", "public, max-age=3600")?;
    }
    h.set("content-type", &content_type)?;
    let status = res.status_code();
    Ok(res.with_headers(h).with_status(status))
}

/// Only `CSP_CONNECT` matters to the page; the bucket's settings may be missing in a page-only check.
fn config_for_page(env: &Env) -> Config {
    let mut c = Config::new(Default::default());
    c.csp_connect = var(env, "CSP_CONNECT")
        .map(|v| v.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    c
}

/// The service: one instance, named `main`, for the whole deployment.
#[durable_object]
pub struct Cloud {
    state: State,
    env: Env,
    router: OnceCell<axum::Router>,
    service: OnceCell<Arc<Service>>,
}

impl Cloud {
    fn service(&self) -> Result<(Arc<Service>, axum::Router)> {
        if let (Some(s), Some(r)) = (self.service.get(), self.router.get()) {
            return Ok((s.clone(), r.clone()));
        }
        let meta = Arc::new(records::SqlRecords::new(self.state.storage().sql())?);
        let service =
            Service::open_with(config(&self.env)?, meta, None, Arc::new(fetch::FetchHttp))
                .map_err(|e| worker::Error::RustError(format!("{e:#}")))?;
        let router = shoebox_cloud::router(service.clone());
        let _ = self.service.set(service.clone());
        let _ = self.router.set(router.clone());
        Ok((service, router))
    }

    /// Make sure the janitor's alarm is set (an `i64` given to `set_alarm` is an offset from now).
    async fn keep_janitor(&self) -> Result<()> {
        let now = Date::now().as_millis() as i64;
        let next = self.state.storage().get_alarm().await?;
        if next.is_none_or(|at| at > now + SWEEP_EVERY_MS) {
            self.state.storage().set_alarm(SWEEP_EVERY_MS).await?;
        }
        Ok(())
    }
}

impl DurableObject for Cloud {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            router: OnceCell::new(),
            service: OnceCell::new(),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let (_, router) = self.service()?;
        self.keep_janitor().await?;
        let url = req.url()?;
        let target = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        };
        let mut builder = axum::http::Request::builder()
            .method(req.method().as_ref())
            .uri(target);
        for (name, value) in req.headers().entries() {
            builder = builder.header(name, value);
        }
        let body = match req.method() {
            Method::Get | Method::Head => vec![],
            _ => req.bytes().await?,
        };
        let request = builder
            .body(axum::body::Body::from(body))
            .map_err(|e| worker::Error::RustError(e.to_string()))?;
        let response = match router.oneshot(request).await {
            Ok(r) => r,
            Err(never) => match never {},
        };
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .map_err(|e| worker::Error::RustError(e.to_string()))?;
        let headers = Headers::new();
        for (name, value) in &parts.headers {
            if let Ok(v) = value.to_str() {
                headers.append(name.as_str(), v)?;
            }
        }
        let res = if bytes.is_empty() {
            Response::empty()?
        } else {
            Response::from_bytes(bytes.to_vec())?
        };
        Ok(res.with_status(parts.status.as_u16()).with_headers(headers))
    }

    async fn alarm(&self) -> Result<Response> {
        let (service, _) = self.service()?;
        service.sweep().await;
        self.state.storage().set_alarm(SWEEP_EVERY_MS).await?;
        Response::empty()
    }
}
