//! `shoebox-cloud`: run the service.

use clap::Parser;
use shoebox_cloud::s3::S3Config;
use shoebox_cloud::{router, Config, Service};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "shoebox-cloud",
    about = "Shoebox Cloud: a store for ciphertext, serving web links and holding libraries."
)]
struct Args {
    /// Where blobs, manifests and library records live.
    #[arg(long, env = "SHOEBOX_CLOUD_DATA", default_value = "./cloud-data")]
    data_dir: PathBuf,
    #[arg(long, env = "SHOEBOX_CLOUD_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// A built viewer page (`apps/web-viewer`), served at `/`.
    #[arg(long, env = "SHOEBOX_CLOUD_VIEWER")]
    viewer_dir: Option<PathBuf>,
    /// Storage a library may use, in GiB.
    #[arg(long, env = "SHOEBOX_QUOTA_GIB", default_value_t = 2)]
    quota_gib: u64,
    /// Bytes a share may serve a day, in GiB.
    #[arg(long, env = "SHOEBOX_SHARE_DAILY_GIB", default_value_t = 1)]
    share_daily_gib: u64,
    /// Refuse libraries that have not registered (a subscription is needed first).
    #[arg(long, env = "SHOEBOX_CLOSED")]
    closed: bool,
    /// Behind a proxy we run: take the client address from `X-Forwarded-For`.
    #[arg(long, env = "SHOEBOX_TRUST_PROXY")]
    trust_proxy: bool,
    /// A host the viewer page may connect to (a relay, for Live links). Repeatable.
    #[arg(
        long = "csp-connect",
        env = "SHOEBOX_CSP_CONNECT",
        value_delimiter = ' '
    )]
    csp_connect: Vec<String>,
    /// Keep the bytes in an S3-compatible bucket at this endpoint (the data directory then holds only
    /// bookkeeping). Browsers are sent to it, so it must be reachable from the internet and allow the viewer's
    /// origin in its CORS rules.
    #[arg(long, env = "SHOEBOX_S3_ENDPOINT")]
    s3_endpoint: Option<url::Url>,
    #[arg(long, env = "SHOEBOX_S3_BUCKET", requires = "s3_endpoint")]
    s3_bucket: Option<String>,
    /// `auto` for Cloudflare R2.
    #[arg(long, env = "SHOEBOX_S3_REGION", default_value = "auto")]
    s3_region: String,
    #[arg(long, env = "SHOEBOX_S3_ACCESS_KEY_ID", hide_env_values = true)]
    s3_access_key_id: Option<String>,
    #[arg(long, env = "SHOEBOX_S3_SECRET_ACCESS_KEY", hide_env_values = true)]
    s3_secret_access_key: Option<String>,
    /// Address the bucket as `https://bucket.endpoint/` instead of `https://endpoint/bucket/`.
    #[arg(long, env = "SHOEBOX_S3_VIRTUAL_HOST")]
    s3_virtual_host: bool,
    /// A token for the operator's `/admin/...` calls (reported shares). Without one there are none.
    #[arg(long, env = "SHOEBOX_ADMIN_TOKEN", hide_env_values = true)]
    admin_token: Option<String>,
}

const GIB: u64 = 1024 * 1024 * 1024;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let mut config = Config::new(args.data_dir);
    config.viewer_dir = args.viewer_dir;
    config.library_quota = args.quota_gib * GIB;
    config.share_daily_bytes = args.share_daily_gib * GIB;
    config.open_registration = !args.closed;
    config.trust_proxy = args.trust_proxy;
    config.csp_connect = args.csp_connect;
    config.admin_token = args.admin_token;
    if let Some(endpoint) = args.s3_endpoint {
        let need = |v: Option<String>, what: &str| {
            v.ok_or_else(|| anyhow::anyhow!("--s3-endpoint needs {what}"))
        };
        config.s3 = Some(S3Config {
            endpoint,
            bucket: need(args.s3_bucket, "--s3-bucket")?,
            region: args.s3_region,
            access_key: need(args.s3_access_key_id, "SHOEBOX_S3_ACCESS_KEY_ID")?,
            secret_key: need(args.s3_secret_access_key, "SHOEBOX_S3_SECRET_ACCESS_KEY")?,
            path_style: !args.s3_virtual_host,
        });
    }
    let service = Service::open(config)?;

    // Expired shares answer "expired" for a week, then are deleted; unfinished uploads give their quota back.
    let janitor = service.clone();
    tokio::spawn(async move {
        loop {
            let n = janitor.sweep().await;
            if n > 0 {
                tracing::info!(deleted = n, "expired shares removed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    println!(
        "shoebox-cloud listening on http://{}",
        listener.local_addr()?
    );
    axum::serve(
        listener,
        router(service).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
