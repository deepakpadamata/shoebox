//! "A new version is available" (R1-07). The site publishes `downloads/release.json`
//! (`site/release.py`); the app and the keeper fetch it now and then and compare its version with
//! their own. Nothing is downloaded or installed: the person is pointed at the download page, where
//! the signed builds are. The request reveals only that someone fetched the file.

use anyhow::{anyhow, bail, Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// The feed until the repository and its site exist (R1-13 sets the real one). Not checked.
pub const PLACEHOLDER_FEED: &str = "https://example.invalid/shoebox/downloads/release.json";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FEED_BYTES: usize = 256 * 1024;

/// What the feed says, as far as an update check cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// The download page: the feed's directory.
    pub page: String,
}

/// Whether `other` is a newer version than `current` (`major.minor.patch`, a `-pre` suffix sorts
/// before the release it precedes). Anything unparsable is never newer.
pub fn newer(current: &str, other: &str) -> bool {
    fn parse(v: &str) -> Option<(Vec<u64>, bool)> {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        let nums = core
            .split('.')
            .map(|p| p.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()?;
        (nums.len() == 3).then_some((nums, pre))
    }
    match (parse(current), parse(other)) {
        // Same numbers: a release is newer than its pre-release, not the other way round.
        (Some((c, cpre)), Some((o, opre))) => o > c || (o == c && cpre && !opre),
        _ => false,
    }
}

fn client() -> Result<reqwest::Client> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.into(),
    };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(TIMEOUT)
        .user_agent(concat!("shoebox/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Fetch `feed` (a `release.json`). The placeholder feed is never contacted.
pub async fn fetch(feed: &str) -> Result<Release> {
    if feed == PLACEHOLDER_FEED {
        bail!("no update feed is configured yet");
    }
    let resp = client()?
        .get(feed)
        .send()
        .await
        .context("fetching the release feed")?
        .error_for_status()?;
    let body = resp.bytes().await?;
    if body.len() > MAX_FEED_BYTES {
        bail!("the release feed is too large");
    }
    let json: serde_json::Value =
        serde_json::from_slice(&body).context("reading the release feed")?;
    let version = json["version"]
        .as_str()
        .ok_or_else(|| anyhow!("the release feed has no version"))?
        .to_string();
    let page = match feed.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/"),
        None => feed.to_string(),
    };
    Ok(Release { version, page })
}

/// [`fetch`], then whether it is newer than this build: `Some` only when it is.
pub async fn check(feed: &str) -> Result<Option<Release>> {
    let release = fetch(feed).await?;
    Ok(newer(env!("CARGO_PKG_VERSION"), &release.version).then_some(release))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.1.0", "0.1.1"));
        assert!(newer("0.1.9", "0.2.0"));
        assert!(newer("0.9.0", "0.10.0"), "numbers, not text");
        assert!(
            newer("0.1.0-rc.1", "0.1.0"),
            "a release follows its pre-release"
        );
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.2.0", "0.1.9"));
        assert!(!newer(
            "0.1.0",
            "0.1.1-rc.1".replace("0.1.1", "0.1.0").as_str()
        ));
        assert!(!newer("0.1.0", "banana"));
        assert!(newer("v0.1.0", "v0.1.1"), "a leading v is fine");
    }
}
