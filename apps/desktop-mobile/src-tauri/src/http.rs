//! A plain HTTP face for [`AppCore`]: `POST /api/<command>`, `GET /events` (server-sent
//! events), `GET /blob/<photo id>?kind=…` (byte ranges honoured) and the built UI as static
//! files. It lets the UI be reviewed in a browser and exercised end to end without a webview.
//! Listens on loopback only; it is a development tool, not part of a shipped app.

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
    app
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
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(core, ui_dist)).await?;
    Ok(())
}
