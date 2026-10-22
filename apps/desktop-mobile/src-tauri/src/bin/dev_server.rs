//! `desktop-dev-server`: runs the app core with the dev HTTP bridge (see `http.rs`).
//!
//!     cargo run -p desktop-mobile --features http --bin desktop-dev-server -- --data-dir /tmp/shoebox-ui
//!
//! Then `npm run dev` in `apps/desktop-mobile` (Vite proxies `/api`, `/events` and `/blob`).

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    data_dir: std::path::PathBuf,
    #[arg(long, default_value = "127.0.0.1:1421")]
    listen: std::net::SocketAddr,
    /// Serve this built UI too (`apps/desktop-mobile/ui/dist`).
    #[arg(long)]
    ui: Option<std::path::PathBuf>,
    /// No relay, no LAN discovery.
    #[arg(long)]
    local_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let opts = if args.local_only {
        desktop_mobile::AppOptions::local_only()
    } else {
        desktop_mobile::AppOptions::default()
    };
    let core = desktop_mobile::AppCore::open(&args.data_dir, opts).await?;
    println!("listening on http://{}", args.listen);
    desktop_mobile::http::serve(core, args.listen, args.ui).await
}
