//! Search by meaning on the keeper (S1-09, S1-11): the text model for its own searches, the
//! image model for indexing (always, up to half the cores), and both offered to the other
//! devices so a phone can index without a download from anywhere else.

use crate::daemon::KeeperOptions;
use shoebox_ml::files::{locate, search_dirs};
use shoebox_ml::{ClipImage, ClipText, PpOcr, SFace, YuNet};
use shoebox_node::{Always, FaceModels, IndexerConfig, LibraryNode};
use shoebox_search::encoder::{ImageEncoder, TextEncoder};
use shoebox_search::models::{ALL, CLIP_IMAGE, CLIP_MODEL, FACES_MODEL, FACE_DETECT, FACE_EMBED};
use shoebox_store::Library;
use std::sync::Arc;
use tracing::{info, warn};

pub(crate) async fn start(ln: &LibraryNode<Library>, opts: &KeeperOptions) {
    if !opts.search_models {
        return;
    }
    ln.set_search_model(CLIP_MODEL);
    let dirs = search_dirs(&[opts.storage.join("models")]);
    let d = dirs.clone();
    ln.set_text_encoder(Arc::new(move || {
        Ok(Arc::new(ClipText::bundled(&d)?) as Arc<dyn TextEncoder>)
    }));
    for file in ALL {
        if let Some(path) = locate(file, &dirs) {
            if let Err(e) = ln.share_model(file, &path).await {
                warn!("could not offer {}: {e:#}", file.name);
            }
        }
    }
    if !opts.index {
        return;
    }
    let Some(path) = locate(&CLIP_IMAGE, &dirs) else {
        info!(
            "{} is not installed: this keeper does not index photos for search by meaning",
            CLIP_IMAGE.name
        );
        return;
    };
    let encoder =
        match tokio::task::spawn_blocking(move || ClipImage::load(&path, CLIP_MODEL)).await {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => return warn!("could not load the image model: {e:#}"),
            Err(e) => return warn!("could not load the image model: {e}"),
        };
    // Faces (S1-17), when both face models are installed.
    if let (Some(d), Some(e)) = (locate(&FACE_DETECT, &dirs), locate(&FACE_EMBED, &dirs)) {
        match (YuNet::load(&d), SFace::load(&e)) {
            (Ok(detector), Ok(embedder)) => ln.set_faces(FaceModels {
                detector: Arc::new(detector),
                embedder: Arc::new(embedder),
                model: FACES_MODEL,
            }),
            (Err(e), _) | (_, Err(e)) => warn!("could not load the face models: {e:#}"),
        }
    }
    // Text in photos (S2-03): PP-OCRv5, when its three files are installed.
    let d = dirs.clone();
    match tokio::task::spawn_blocking(move || PpOcr::locate(&d)).await {
        Ok(Ok(Some(e))) => ln.set_ocr(Arc::new(e)),
        Ok(Ok(None)) => {
            info!("the text models are not installed: this keeper reads no text in photos")
        }
        Ok(Err(e)) => warn!("could not load the text models: {e:#}"),
        Err(e) => warn!("could not load the text models: {e}"),
    }
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    let cfg = IndexerConfig {
        always_on: true,
        threads: opts.max_threads.unwrap_or((cores / 2).max(1)),
        ..Default::default()
    };
    info!("indexing photos for search with {} thread(s)", cfg.threads);
    ln.start_indexer(
        cfg,
        Arc::new(encoder) as Arc<dyn ImageEncoder>,
        Arc::new(Always),
    );
}
