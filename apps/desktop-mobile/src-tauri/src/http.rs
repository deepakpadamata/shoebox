//! A plain HTTP face for [`AppCore`]: `POST /api/<command>`, `GET /events` (server-sent
//! events), `GET /blob/<photo id>?kind=…` (byte ranges honoured) and the built UI as static
//! files. It lets the UI be reviewed in a browser and exercised end to end without a webview.
//! Listens on loopback only; it is a development tool, not part of a shipped app.
//!
//! A web page the developer has open could still reach a loopback port, so every request must
//! name a loopback host (stops DNS rebinding), a browser `Origin` must be loopback too (stops
//! cross-site requests), and commands must be sent as JSON (a cross-site form or `text/plain`
//! post is refused instead of running without a CORS preflight).

use crate::commands::{dispatch, user_message};
use crate::core::AppCore;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::services::{ServeDir, ServeFile};

pub fn router(core: Arc<AppCore>, ui_dist: Option<PathBuf>) -> Router {
    let mut app = Router::new()
        .route("/api/{cmd}", post(api))
        .route("/events", get(events))
        .route("/blob/{id}", get(blob))
        .with_state(core);
    if let Some(dist) = ui_dist {
        let index = dist.join("index.html");
        app = app.fallback_service(ServeDir::new(dist).fallback(ServeFile::new(index)));
    }
    app.layer(axum::middleware::from_fn(loopback_only))
}

/// `localhost`, `127.0.0.1` or `[::1]`, with or without a port.
fn is_loopback_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The origin's host part, e.g. `localhost:5173` from `http://localhost:5173`.
fn origin_host(origin: &str) -> Option<&str> {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))?;
    Some(rest.split('/').next().unwrap_or(rest))
}

async fn loopback_only(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let headers = req.headers();
    let text = |name| {
        headers
            .get(name)
            .and_then(|v: &header::HeaderValue| v.to_str().ok())
    };
    let host_ok =
        headers.get(header::HOST).is_none() || text(header::HOST).is_some_and(is_loopback_host);
    let origin_ok = headers.get(header::ORIGIN).is_none()
        || text(header::ORIGIN)
            .and_then(origin_host)
            .is_some_and(is_loopback_host);
    if !host_ok || !origin_ok {
        return (StatusCode::FORBIDDEN, "loopback pages only").into_response();
    }
    if req.uri().path().starts_with("/api/") {
        let json = text(header::CONTENT_TYPE)
            .and_then(|c| c.split(';').next())
            .is_some_and(|c| c.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "commands are sent as application/json",
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn api(
    State(core): State<Arc<AppCore>>,
    Path(cmd): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let args: Value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("bad request body: {e}") })),
                )
                    .into_response()
            }
        }
    };
    match dispatch(&core, &cmd, args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": user_message(&e) })),
        )
            .into_response(),
    }
}

async fn events(
    State(core): State<Arc<AppCore>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(core.events()).filter_map(|e| {
        let e = e.ok()?;
        Some(Ok(Event::default().data(serde_json::to_string(&e).ok()?)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn parse_range(headers: &HeaderMap) -> Option<(u64, Option<u64>)> {
    let v = headers.get(header::RANGE)?.to_str().ok()?;
    let spec = v.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let first: u64 = a.parse().ok()?;
    let last = if b.is_empty() {
        None
    } else {
        Some(b.parse().ok()?)
    };
    Some((first, last))
}

async fn blob(
    State(core): State<Arc<AppCore>>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let kind = q.get("kind").map(String::as_str).unwrap_or("thumb");
    match core.blob(&id, kind, parse_range(&headers)).await {
        Ok(Some(b)) => {
            let mut resp = Response::builder()
                .header(header::CONTENT_TYPE, &b.content_type)
                .header(header::ETAG, format!("\"{}\"", b.etag))
                .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                .header(header::ACCEPT_RANGES, "bytes");
            match b.range {
                Some((first, last)) => {
                    resp = resp.status(StatusCode::PARTIAL_CONTENT).header(
                        header::CONTENT_RANGE,
                        format!("bytes {first}-{last}/{}", b.total),
                    );
                }
                None => resp = resp.status(StatusCode::OK),
            }
            resp.body(Body::from(b.bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Ok(None) => (StatusCode::NOT_FOUND, "not on this device").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, user_message(&e)).into_response(),
    }
}

/// Serve `core` on `addr` until the process ends.
pub async fn serve(
    core: Arc<AppCore>,
    addr: std::net::SocketAddr,
    ui_dist: Option<PathBuf>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        addr.ip().is_loopback(),
        "the dev bridge listens on loopback only, not {addr}"
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(core, ui_dist)).await?;
    Ok(())
}
