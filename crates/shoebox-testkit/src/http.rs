//! A tiny HTTP file server for tests that download (S2-02): it serves one body at any path on
//! 127.0.0.1, answers `Range: bytes=N-` like a real server, and can be told to drop the first
//! response part-way or to ignore ranges, so resuming can be tested.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What to serve and how badly to behave. See [`FileServer::serve`].
pub struct FileServerBuilder {
    body: Arc<Vec<u8>>,
    cut_first_at: Option<usize>,
    ignore_ranges: bool,
}

impl FileServerBuilder {
    /// The first response announces the whole body but sends only this many bytes of it, then
    /// closes the connection.
    pub fn cut_first_response_at(mut self, bytes: usize) -> Self {
        self.cut_first_at = Some(bytes);
        self
    }

    /// Answer every request with the whole body, as a server that cannot resume does.
    pub fn ignoring_ranges(mut self) -> Self {
        self.ignore_ranges = true;
        self
    }

    pub async fn start(self) -> FileServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let nth = {
                    let mut seen = log.lock().unwrap();
                    seen.push(None);
                    seen.len() - 1
                };
                tokio::spawn(answer(
                    stream,
                    self.body.clone(),
                    log.clone(),
                    nth,
                    self.cut_first_at.filter(|_| nth == 0),
                    self.ignore_ranges,
                ));
            }
        });
        FileServer { addr, seen, task }
    }
}

/// A running server; stops when dropped.
pub struct FileServer {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Option<u64>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FileServer {
    /// Serve `body`, whatever the path.
    pub fn serve(body: Vec<u8>) -> FileServerBuilder {
        FileServerBuilder {
            body: Arc::new(body),
            cut_first_at: None,
            ignore_ranges: false,
        }
    }

    /// An address to fetch the body from.
    pub fn url(&self) -> String {
        format!("http://{}/model.onnx", self.addr)
    }

    /// One entry per request so far: the start of its `Range`, if it had one.
    pub fn requests(&self) -> Vec<Option<u64>> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for FileServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn answer(
    mut stream: TcpStream,
    body: Arc<Vec<u8>>,
    seen: Arc<Mutex<Vec<Option<u64>>>>,
    nth: usize,
    cut_at: Option<usize>,
    ignore_ranges: bool,
) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head).to_lowercase();
    let range = text
        .lines()
        .find_map(|l| l.strip_prefix("range: bytes="))
        .and_then(|r| r.trim().strip_suffix('-'))
        .and_then(|n| n.parse::<u64>().ok());
    seen.lock().unwrap()[nth] = range;
    let len = body.len();
    let (status, start) = match range {
        Some(n) if !ignore_ranges => ("206 Partial Content", (n as usize).min(len)),
        _ => ("200 OK", 0),
    };
    let mut reply = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        len - start
    );
    if start > 0 || status.starts_with("206") {
        reply.push_str(&format!(
            "Content-Range: bytes {start}-{}/{len}\r\n",
            len.saturating_sub(1)
        ));
    }
    reply.push_str("\r\n");
    if stream.write_all(reply.as_bytes()).await.is_err() {
        return;
    }
    let rest = &body[start..];
    match cut_at {
        // Drop the connection with the body unfinished.
        Some(n) => {
            let _ = stream.write_all(&rest[..n.min(rest.len())]).await;
        }
        None => {
            let _ = stream.write_all(rest).await;
            let _ = stream.shutdown().await;
        }
    }
}
