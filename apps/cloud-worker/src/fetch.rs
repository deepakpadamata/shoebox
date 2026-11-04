//! The bucket's own requests (a size check, a manifest, a delete) through the Workers `fetch`.

use anyhow::anyhow;
use futures::future::BoxFuture;
use shoebox_cloud::s3::{Http, Reply};
use url::Url;
use worker::send::SendFuture;
use worker::{js_sys, Fetch, Method, Request, RequestInit};

pub struct FetchHttp;

impl Http for FetchHttp {
    fn send(
        &self,
        method: &'static str,
        url: Url,
        body: Option<Vec<u8>>,
    ) -> BoxFuture<'_, anyhow::Result<Reply>> {
        // A Worker runs on one thread; the service's handlers only need the future to be `Send` by type.
        Box::pin(SendFuture::new(async move {
            let mut init = RequestInit::new();
            init.with_method(Method::from(method.to_string()));
            if let Some(b) = body {
                init.with_body(Some(js_sys::Uint8Array::from(b.as_slice()).into()));
            }
            let req = Request::new_with_init(url.as_str(), &init).map_err(|e| anyhow!("{e}"))?;
            let res = Fetch::Request(req)
                .send()
                .await
                .map_err(|e| anyhow!("asking the bucket: {e}"))?;
            let content_length = res
                .headers()
                .get("content-length")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok());
            Ok(Reply {
                status: res.status_code(),
                content_length,
            })
        }))
    }
}
