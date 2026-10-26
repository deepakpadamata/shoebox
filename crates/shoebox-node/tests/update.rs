//! R1-07: a build learns about a newer release from the site's `release.json` (served here by a
//! tiny local server, as the real site will once it exists).

use shoebox_node::update;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serve `body` as `/downloads/release.json` for every request; returns the feed URL.
async fn feed(body: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/downloads/release.json")
}

#[tokio::test]
async fn a_newer_release_on_the_feed_is_reported_with_its_download_page() {
    let url = feed(r#"{"version":"99.0.0","files":[]}"#.into()).await;
    let r = update::check(&url).await.unwrap().expect("99.0.0 is newer");
    assert_eq!(r.version, "99.0.0");
    assert_eq!(r.page, url.trim_end_matches("release.json"));
}

#[tokio::test]
async fn the_same_or_an_older_release_is_not() {
    let url = feed(format!(r#"{{"version":"{}"}}"#, env!("CARGO_PKG_VERSION"))).await;
    assert_eq!(update::check(&url).await.unwrap(), None);
    let url = feed(r#"{"version":"0.0.1"}"#.into()).await;
    assert_eq!(update::check(&url).await.unwrap(), None);
}

#[tokio::test]
async fn a_broken_feed_is_an_error_and_the_placeholder_is_never_contacted() {
    let url = feed("not json".into()).await;
    assert!(update::check(&url).await.is_err());
    let err = update::check(update::PLACEHOLDER_FEED).await.unwrap_err();
    assert!(format!("{err:#}").contains("no update feed"), "{err:#}");
}

/// Needs the internet: the TLS setup reaches a real HTTPS site (a 404 is fine, a TLS error is not).
/// Run with `cargo test -p shoebox-node --test update -- --ignored`.
#[tokio::test]
#[ignore]
async fn https_works_against_a_real_site() {
    let err = update::fetch("https://example.com/shoebox-release.json")
        .await
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("404"), "expected an HTTP 404, got: {msg}");
}
