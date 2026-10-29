//! `desktop-dev-server`: runs the app core with the dev HTTP bridge (see `http.rs`).
//!
//!     cargo run -p desktop-mobile --features http --bin desktop-dev-server -- --data-dir /tmp/shoebox-ui
//!
//! Then `npm run dev` in `apps/desktop-mobile` (Vite proxies `/api`, `/events` and `/blob`).

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "desktop-dev-server", version)]
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
    /// Look for new releases here (a `release.json`), e.g. a local test feed.
    #[arg(long)]
    update_feed: Option<String>,
    /// Pretend this folder is the phone's gallery: every image and video under it is in the
    /// camera roll (a fake photo source), to try backup and its folder choice in a browser.
    #[arg(long)]
    fake_camera_roll: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    desktop_mobile::diagnostics::init_logging();
    let args = Args::parse();
    let opts = if args.local_only {
        desktop_mobile::AppOptions::local_only()
    } else {
        desktop_mobile::AppOptions::default()
    };
    let mut opts = opts;
    if let Some(feed) = args.update_feed {
        opts.update_feed = feed;
    }
    let core = desktop_mobile::AppCore::open(&args.data_dir, opts).await?;
    if let Some(roll) = args.fake_camera_roll {
        use desktop_mobile::photos_source::FakePhotoSource;
        let source = std::sync::Arc::new(FakePhotoSource::new(args.data_dir.join("fake-picks")));
        let mut todo = vec![roll];
        let mut n = 0;
        while let Some(dir) = todo.pop() {
            for e in std::fs::read_dir(&dir)?.flatten() {
                let p = e.path();
                let media = p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                    ["jpg", "jpeg", "png", "heic", "mp4"].contains(&x.to_lowercase().as_str())
                });
                if p.is_dir() {
                    todo.push(p);
                } else if media {
                    n += 1;
                    source.put(&format!("fake{n}"), &p, n);
                }
            }
        }
        core.set_photo_source(source);
    }
    println!("listening on http://{}", args.listen);
    desktop_mobile::http::serve(core, args.listen, args.ui).await
}
