//! Downloading a search model file from its published address (S2-02).
//!
//! A device that finds the image or face model neither in its own folders nor on another device
//! of its library (a phone on its own, say) fetches it once from the release the file's `url`
//! names. Nothing about the device or the library is sent: the request is a plain `GET` with the
//! app's name and version as the user agent.
//!
//! The address is not trusted for anything but the bytes: every file is checked against the size
//! and BLAKE3 hash compiled into the app, and nothing is kept under its real name until it
//! matches. An interrupted download resumes from the `.part` file with a `Range` request.

use anyhow::{bail, Context, Result};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use shoebox_search::models::ModelFile;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// A connection that sends nothing for this long is given up; the next try resumes.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Called with the bytes of the file in hand and its size, as they arrive.
pub type Progress<'a> = &'a (dyn Fn(u64, u64) + Send + Sync);

/// Download `file` from its published address into `dir`, returning the path of the finished,
/// verified file (`dir/<name>`).
pub async fn download_model(
    file: &ModelFile,
    dir: &Path,
    progress: Progress<'_>,
) -> Result<PathBuf> {
    download_model_from(file.url, file, dir, progress).await
}

/// [`download_model`] from another address (tests).
pub async fn download_model_from(
    url: &str,
    file: &ModelFile,
    dir: &Path,
    progress: Progress<'_>,
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    let part = dir.join(format!("{}.part", file.name));
    let client = crate::update::https_builder()?
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()?;
    // The second pass is for a server that cannot resume from where we are.
    for pass in 0..2 {
        let have = match tokio::fs::metadata(&part).await {
            Ok(m) if m.len() <= file.size => m.len(),
            Ok(_) => {
                let _ = tokio::fs::remove_file(&part).await;
                0
            }
            Err(_) => 0,
        };
        if have == file.size {
            break; // a run that got every byte but was stopped before it checked them
        }
        let mut request = client.get(url);
        if have > 0 {
            request = request.header(RANGE, format!("bytes={have}-"));
        }
        let mut resp = request
            .send()
            .await
            .with_context(|| format!("fetching {}", file.name))?;
        let status = resp.status();
        let resumes = status == StatusCode::PARTIAL_CONTENT
            && have > 0
            && resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with(&format!("bytes {have}-")));
        let (mut out, mut done) = if resumes {
            let out = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .await?;
            (out, have)
        } else if status == StatusCode::OK {
            (tokio::fs::File::create(&part).await?, 0)
        } else if (status == StatusCode::RANGE_NOT_SATISFIABLE
            || status == StatusCode::PARTIAL_CONTENT)
            && have > 0
            && pass == 0
        {
            let _ = tokio::fs::remove_file(&part).await;
            continue;
        } else {
            bail!("{}: the server answered {status}", file.name);
        };
        loop {
            let chunk = match resp.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => {
                    // What arrived stays on disk, so the next try resumes after it.
                    out.flush().await?;
                    return Err(e)
                        .with_context(|| format!("downloading {} (it will resume)", file.name));
                }
            };
            done += chunk.len() as u64;
            if done > file.size {
                drop(out);
                let _ = tokio::fs::remove_file(&part).await;
                bail!(
                    "{} is larger than the {} bytes expected",
                    file.name,
                    file.size
                );
            }
            out.write_all(&chunk).await?;
            progress(done, file.size);
        }
        out.flush().await?;
        break;
    }
    let (len, hash) = {
        let part = part.clone();
        tokio::task::spawn_blocking(move || -> Result<(u64, [u8; 32])> {
            let mut f = std::fs::File::open(&part)?;
            let mut h = blake3::Hasher::new();
            let n = std::io::copy(&mut f, &mut h)?;
            Ok((n, *h.finalize().as_bytes()))
        })
        .await??
    };
    if len != file.size || hash != file.blake3 {
        let _ = tokio::fs::remove_file(&part).await;
        bail!(
            "what was downloaded for {} is not the model this app expects (kept nothing)",
            file.name
        );
    }
    let target = dir.join(file.name);
    tokio::fs::rename(&part, &target).await?;
    Ok(target)
}
