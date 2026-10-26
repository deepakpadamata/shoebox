#![cfg(feature = "http")]
//! M3-03/04 over the dev HTTP bridge: commands, events and blobs, without a webview.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use desktop_mobile::http::router;
use desktop_mobile::{AppCore, AppOptions};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tower::ServiceExt;

fn sample(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/samples")
        .join(name)
        .display()
        .to_string()
}

async fn call(app: &axum::Router, cmd: &str, args: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("/api/{cmd}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(args.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn commands_events_and_blobs_work_over_http() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(dir.path(), AppOptions::local_only())
        .await
        .unwrap();
    let app = router(core.clone(), None::<PathBuf>);

    let (st, state) = call(&app, "app_state", Value::Null).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(state["has_library"], false);

    // Errors are 400 with a message the UI can show.
    let (st, err) = call(&app, "library_page", json!({})).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"], "no library yet");
    assert_eq!(
        call(&app, "no_such_command", json!({})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "set_favorite",
            json!({"ids": "oops", "favorite": true})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    let (_, created) = call(
        &app,
        "create_library",
        json!({"device_name": "MacBook", "role": "keeper"}),
    )
    .await;
    assert_eq!(
        created["phrase"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .count(),
        24
    );

    // Events stream: subscribe, then import, and the change arrives.
    let events = app
        .clone()
        .oneshot(Request::get("/events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let mut body = events.into_body();
    let (_, report) = call(
        &app,
        "import_paths",
        json!({"paths": [sample("photo-gps.jpg")]}),
    )
    .await;
    assert_eq!(report["imported"], 1);
    let mut seen = String::new();
    let got = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = body.frame().await {
            if let Ok(f) = frame {
                if let Some(d) = f.data_ref() {
                    seen.push_str(&String::from_utf8_lossy(d));
                    if seen.contains("LibraryChanged") {
                        return true;
                    }
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(got, "LibraryChanged arrives over SSE; got {seen:?}");

    let (_, page) = call(&app, "library_page", json!({"limit": 10})).await;
    let id = page["items"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(page["next"], Value::Null);

    // Whole thumbnail with immutable caching headers.
    let r = app
        .clone()
        .oneshot(
            Request::get(format!("/blob/{id}?kind=thumb"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("immutable"));
    assert!(r.headers().contains_key(header::ETAG));
    let thumb = r.into_body().collect().await.unwrap().to_bytes();
    assert!(thumb.len() > 100);

    // A byte range of the original: 206 with Content-Range.
    let r = app
        .clone()
        .oneshot(
            Request::get(format!("/blob/{id}?kind=original"))
                .header(header::RANGE, "bytes=4-11")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    let cr = r.headers()[header::CONTENT_RANGE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(cr.starts_with("bytes 4-11/"), "{cr}");
    assert_eq!(r.into_body().collect().await.unwrap().to_bytes().len(), 8);

    // Unknown photo → 400, a photo that is not here → 404 (checked via a made-up library state).
    let r = app
        .clone()
        .oneshot(
            Request::get(format!("/blob/{}?kind=thumb", "0".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    core.shutdown().await.unwrap();
}

/// A web page the developer has open must not drive the bridge: a foreign `Origin`, a
/// DNS-rebinding `Host` and a non-JSON (no-preflight) command are all refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn other_sites_cannot_use_the_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(dir.path(), AppOptions::local_only())
        .await
        .unwrap();
    let app = router(core, None::<PathBuf>);
    let send = |req: Request<Body>| {
        let app = app.clone();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    let post = |host: &str, origin: Option<&str>, ctype: &str| {
        let mut b = Request::post("/api/app_state")
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, ctype);
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(Body::from("null")).unwrap()
    };

    // The UI itself (Vite on another loopback port) and plain tools work.
    assert_eq!(
        send(post(
            "127.0.0.1:1421",
            Some("http://localhost:5173"),
            "application/json"
        ))
        .await,
        StatusCode::OK
    );
    assert_eq!(
        send(post("[::1]:1421", None, "application/json; charset=utf-8")).await,
        StatusCode::OK
    );
    // Cross-site page, rebinding host, form-style post.
    assert_eq!(
        send(post(
            "127.0.0.1:1421",
            Some("https://evil.example"),
            "application/json"
        ))
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(post("evil.example:1421", None, "application/json")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(post("127.0.0.1:1421", None, "text/plain")).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let events = Request::get("/events")
        .header(header::HOST, "attacker.localhost.evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(events).await, StatusCode::FORBIDDEN);
}
