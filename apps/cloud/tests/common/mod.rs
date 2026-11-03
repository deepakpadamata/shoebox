//! A stand-in for an S3-compatible bucket, on a real socket, for the tests. It checks every request's presigned
//! signature by signing the same request again with the same credentials (so a URL used with another method, key,
//! length or after it ran out is refused, as S3 refuses it), and refuses a request that also carries an
//! `Authorization` header, as S3 does.

#![allow(dead_code)]

use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use rusty_s3::actions::{DeleteObject, GetObject, HeadObject, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use shoebox_cloud::s3::S3Config;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Clone)]
pub struct FakeS3 {
    pub config: S3Config,
    /// key -> bytes
    pub objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    /// Requests refused, with why.
    pub refused: Arc<Mutex<Vec<String>>>,
}

impl FakeS3 {
    pub fn keys(&self) -> Vec<String> {
        self.objects.lock().unwrap().keys().cloned().collect()
    }
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.objects.lock().unwrap().get(key).cloned()
    }
    pub fn refused(&self) -> Vec<String> {
        self.refused.lock().unwrap().clone()
    }
}

pub async fn start(clock: Clock) -> FakeS3 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = S3Config {
        endpoint: format!("http://{addr}").parse().unwrap(),
        bucket: "photos".into(),
        region: "auto".into(),
        access_key: "test-key-id".into(),
        secret_key: "test-secret".into(),
        path_style: true,
    };
    let fake = FakeS3 {
        config: config.clone(),
        objects: Arc::default(),
        refused: Arc::default(),
    };
    let state = (fake.clone(), clock);
    let app = axum::Router::new().fallback(handle).with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    fake
}

fn x_amz_date(text: &str) -> Option<jiff::Timestamp> {
    jiff::civil::DateTime::strptime("%Y%m%dT%H%M%SZ", text)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp())
}

async fn handle(
    axum::extract::State((fake, clock)): axum::extract::State<(FakeS3, Clock)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let refuse = |status: StatusCode, why: String| {
        fake.refused
            .lock()
            .unwrap()
            .push(format!("{method} {}: {why}", uri.path()));
        (status, why).into_response()
    };
    if headers.contains_key("authorization") {
        return refuse(
            StatusCode::BAD_REQUEST,
            "only one auth mechanism allowed".into(),
        );
    }
    let Some(key) = uri
        .path()
        .strip_prefix(&format!("/{}/", fake.config.bucket))
    else {
        return refuse(StatusCode::NOT_FOUND, "no such bucket".into());
    };
    let key = key.to_string();
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    let (Some(date), Some(expires), Some(signed), Some(sig)) = (
        query.get("X-Amz-Date").and_then(|d| x_amz_date(d)),
        query
            .get("X-Amz-Expires")
            .and_then(|e| e.parse::<u64>().ok()),
        query.get("X-Amz-SignedHeaders"),
        query.get("X-Amz-Signature"),
    ) else {
        return refuse(StatusCode::FORBIDDEN, "not presigned".into());
    };
    let now = clock();
    let start = date.as_millisecond();
    if now < start - 60_000 || now > start + expires as i64 * 1000 {
        return refuse(StatusCode::FORBIDDEN, "request has expired".into());
    }

    // Sign the request as received; the signature must come out the same.
    let bucket = Bucket::new(
        fake.config.endpoint.clone(),
        UrlStyle::Path,
        fake.config.bucket.clone(),
        fake.config.region.clone(),
    )
    .unwrap();
    let creds = Credentials::new(
        fake.config.access_key.clone(),
        fake.config.secret_key.clone(),
    );
    let signed_values: Vec<(String, String)> = signed
        .split(';')
        .filter(|h| *h != "host")
        .map(|h| {
            (
                h.to_string(),
                headers
                    .get(h)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    let ttl = Duration::from_secs(expires);
    let again = match method {
        Method::GET => sign(
            GetObject::new(&bucket, Some(&creds), &key),
            &signed_values,
            ttl,
            &date,
        ),
        Method::HEAD => sign(
            HeadObject::new(&bucket, Some(&creds), &key),
            &signed_values,
            ttl,
            &date,
        ),
        Method::PUT => sign(
            PutObject::new(&bucket, Some(&creds), &key),
            &signed_values,
            ttl,
            &date,
        ),
        Method::DELETE => sign(
            DeleteObject::new(&bucket, Some(&creds), &key),
            &signed_values,
            ttl,
            &date,
        ),
        _ => return refuse(StatusCode::METHOD_NOT_ALLOWED, "method".into()),
    };
    let expected = again
        .query_pairs()
        .find(|(k, _)| k == "X-Amz-Signature")
        .map(|(_, v)| v.into_owned());
    if expected.as_deref() != Some(sig.as_str()) {
        return refuse(StatusCode::FORBIDDEN, "SignatureDoesNotMatch".into());
    }

    let mut objects = fake.objects.lock().unwrap();
    match method {
        Method::GET => match objects.get(&key) {
            Some(b) => (StatusCode::OK, b.clone()).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Method::HEAD => match objects.get(&key) {
            Some(b) => (StatusCode::OK, [("content-length", b.len().to_string())]).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Method::DELETE => {
            objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        Method::PUT => {
            drop(objects);
            let declared: Option<usize> = headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            let bytes = body.collect().await.unwrap().to_bytes().to_vec();
            if declared != Some(bytes.len()) {
                return refuse(StatusCode::BAD_REQUEST, "IncompleteBody".into());
            }
            fake.objects.lock().unwrap().insert(key, bytes);
            StatusCode::OK.into_response()
        }
        _ => unreachable!(),
    }
}

fn sign<'a, A: S3Action<'a>>(
    mut action: A,
    headers: &'a [(String, String)],
    ttl: Duration,
    at: &jiff::Timestamp,
) -> url::Url {
    for (k, v) in headers {
        action.headers_mut().insert(k.as_str(), v.as_str());
    }
    action.sign_with_time(ttl, at)
}

/// An HTTP client for the tests (rustls needs its crypto chosen once).
pub fn http() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}
