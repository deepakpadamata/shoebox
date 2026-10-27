//! Video originals over a loopback HTTP server (used on Android).
//!
//! Android's WebView applies a request's byte range to whatever a custom-scheme handler returns:
//! it skips the first N bytes of the response itself. A correct partial answer from
//! `shoebox://` is therefore cut again and fails, so a video plays only while every request
//! starts at byte 0 (one piece). Real HTTP has no such rule. The server listens on 127.0.0.1
//! only and every path starts with a random 256-bit token: other apps on the phone can reach the
//! port but cannot name a blob. It answers `GET`/`HEAD /<token>/blob/<photo id>?kind=…`, the
//! same blobs and kinds as `shoebox://`, one request per connection.

use crate::core::AppCore;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Request heads longer than this are refused.
const MAX_HEAD: usize = 16 * 1024;

/// Start the server; returns the base URL (`http://127.0.0.1:<port>/<token>`).
pub async fn start(core: Arc<AppCore>) -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| anyhow!("no randomness: {e}"))?;
    let token = hex::encode(secret);
    let base = format!("http://127.0.0.1:{port}/{token}");
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            let (core, token) = (core.clone(), token.clone());
            tokio::spawn(async move {
                if let Err(e) = handle(&core, &token, conn).await {
                    tracing::debug!("media request: {e:#}");
                }
            });
        }
    });
    Ok(base)
}

async fn handle(core: &Arc<AppCore>, token: &str, conn: TcpStream) -> Result<()> {
    let mut conn = BufReader::new(conn);
    let (mut head, mut line) = (Vec::<String>::new(), String::new());
    let mut size = 0;
    loop {
        line.clear();
        let n = conn.read_line(&mut line).await?;
        size += n;
        if n == 0 || size > MAX_HEAD {
            return Err(anyhow!("incomplete or oversized request head"));
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        head.push(l.to_string());
    }
    let mut parts = head.first().map(|l| l.split(' ')).into_iter().flatten();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let range = head.iter().skip(1).find_map(|h| {
        let (name, value) = h.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("range")
            .then(|| crate::blob::parse_range(value.trim()))
            .flatten()
    });
    let started = std::time::Instant::now();
    let reply = answer(core, token, method, target, range).await;
    tracing::debug!(
        ?range,
        status = reply.status,
        bytes = reply.body.len(),
        ms = started.elapsed().as_millis() as u64,
        "media request"
    );
    let conn = conn.get_mut();
    conn.write_all(&reply.head()).await?;
    if method != "HEAD" {
        conn.write_all(&reply.body).await?;
    }
    conn.shutdown().await?;
    Ok(())
}

struct Reply {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn plain(status: &'static str, text: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type", "text/plain; charset=utf-8".into())],
            body: text.as_bytes().to_vec(),
        }
    }

    fn head(&self) -> Vec<u8> {
        let mut h = format!("HTTP/1.1 {}\r\n", self.status);
        for (name, value) in &self.headers {
            h.push_str(&format!("{name}: {value}\r\n"));
        }
        h.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
            self.body.len()
        ));
        h.into_bytes()
    }
}

async fn answer(
    core: &Arc<AppCore>,
    token: &str,
    method: &str,
    target: &str,
    range: Option<(u64, Option<u64>)>,
) -> Reply {
    if method != "GET" && method != "HEAD" {
        return Reply::plain("405 Method Not Allowed", "only GET");
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let id = path
        .strip_prefix('/')
        .and_then(|p| p.split_once('/'))
        .filter(|(t, _)| same(t.as_bytes(), token.as_bytes()))
        .and_then(|(_, rest)| rest.strip_prefix("blob/"));
    let Some(id) = id else {
        return Reply::plain("404 Not Found", "not found");
    };
    let kind = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("kind="))
        .unwrap_or("original");
    match core.blob(id, kind, range).await {
        Ok(Some(b)) => {
            let mut headers = vec![
                ("Content-Type", b.content_type.clone()),
                ("ETag", format!("\"{}\"", b.etag)),
                ("Accept-Ranges", "bytes".into()),
                // The token is new on every start, so a cached copy would never be used again; and
                // the WebView's cache stalled a long video on cached pieces (seen on a Galaxy S25).
                ("Cache-Control", "no-store".into()),
            ];
            if matches!(range, Some((first, _)) if first >= b.total && b.total > 0) {
                headers.push(("Content-Range", format!("bytes */{}", b.total)));
                return Reply {
                    status: "416 Range Not Satisfiable",
                    headers,
                    body: vec![],
                };
            }
            let status = match b.range {
                Some((first, last)) => {
                    headers.push(("Content-Range", format!("bytes {first}-{last}/{}", b.total)));
                    "206 Partial Content"
                }
                None => "200 OK",
            };
            Reply {
                status,
                headers,
                body: b.bytes,
            }
        }
        Ok(None) => Reply::plain("404 Not Found", "not on this device"),
        Err(e) => Reply::plain("400 Bad Request", &crate::commands::user_message(&e)),
    }
}

/// Compare without stopping at the first difference (the token is a secret).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
