//! A model that can be dropped from memory while it is not needed and loaded again on its next
//! use. The image and face models take hundreds of MB once loaded, and an indexer that has caught
//! up may not need them again for hours.

use crate::{ClipImage, SFace, YuNet};
use anyhow::Result;
use shoebox_core::model::ModelId;
use shoebox_search::encoder::{EncodeError, ImageEncoder, Rgb};
use shoebox_search::faces::{Detection, FaceDetector, FaceEmbedder};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Loader<T> = Box<dyn Fn() -> Result<T> + Send + Sync>;

/// `T`, loaded by `loader` when used and dropped by [`Reloading::unload`] (the traits' `release`).
pub struct Reloading<T> {
    loader: Loader<T>,
    loaded: Mutex<Option<Arc<T>>>,
    /// What the model says about itself, kept so asking does not load it.
    model: ModelId,
    dims: usize,
}

impl<T> Reloading<T> {
    /// Load once now, so a missing or bad file fails here, and keep `loader` for later.
    fn new(
        loader: impl Fn() -> Result<T> + Send + Sync + 'static,
        describe: impl FnOnce(&T) -> (ModelId, usize),
    ) -> Result<Self> {
        let first = loader()?;
        let (model, dims) = describe(&first);
        Ok(Self {
            loader: Box::new(loader),
            loaded: Mutex::new(Some(Arc::new(first))),
            model,
            dims,
        })
    }

    /// The model, loading it again if it was released. Concurrent callers share one load.
    fn get(&self) -> Result<Arc<T>, EncodeError> {
        let mut slot = self.loaded.lock().unwrap();
        if let Some(m) = &*slot {
            return Ok(m.clone());
        }
        let m = Arc::new(
            (self.loader)().map_err(|e| EncodeError(format!("loading the model again: {e:#}")))?,
        );
        *slot = Some(m.clone());
        Ok(m)
    }

    /// Drop the model; calls in flight finish with the copy they hold.
    pub fn unload(&self) {
        self.loaded.lock().unwrap().take();
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.lock().unwrap().is_some()
    }
}

impl Reloading<ClipImage> {
    pub fn clip_image(path: PathBuf, model: ModelId) -> Result<Self> {
        Self::new(
            move || ClipImage::load(&path, model),
            |m| (m.model(), m.dims()),
        )
    }
}

impl Reloading<YuNet> {
    pub fn yunet(path: PathBuf) -> Result<Self> {
        Self::new(move || YuNet::load(&path), |_| (ModelId([0; 32]), 0))
    }
}

impl Reloading<SFace> {
    pub fn sface(path: PathBuf) -> Result<Self> {
        Self::new(move || SFace::load(&path), |m| (ModelId([0; 32]), m.dims()))
    }
}

impl ImageEncoder for Reloading<ClipImage> {
    fn model(&self) -> ModelId {
        self.model
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn encode_image(&self, image: &Rgb) -> Result<Vec<f32>, EncodeError> {
        self.get()?.encode_image(image)
    }
    fn release(&self) {
        self.unload();
    }
}

impl FaceDetector for Reloading<YuNet> {
    fn detect(&self, image: &Rgb) -> Result<Vec<Detection>, EncodeError> {
        self.get()?.detect(image)
    }
    fn release(&self) {
        self.unload();
    }
}

impl FaceEmbedder for Reloading<SFace> {
    fn dims(&self) -> usize {
        self.dims
    }
    fn embed(&self, image: &Rgb, landmarks: &[[f32; 2]; 5]) -> Result<Vec<f32>, EncodeError> {
        self.get()?.embed(image, landmarks)
    }
    fn release(&self) {
        self.unload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_released_model_loads_again_on_its_next_use_and_not_before() {
        let loads = Arc::new(AtomicUsize::new(0));
        let l = loads.clone();
        let r = Reloading::new(
            move || Ok(l.fetch_add(1, Ordering::SeqCst)),
            |_| (ModelId([7; 32]), 3),
        )
        .unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        r.get().unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1, "kept while loaded");
        r.unload();
        assert!(!r.is_loaded());
        assert_eq!(
            (r.model, r.dims),
            (ModelId([7; 32]), 3),
            "known without loading"
        );
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(*r.get().unwrap(), 1);
        assert!(r.is_loaded());
    }

    #[test]
    fn a_model_that_cannot_load_fails_at_once() {
        assert!(
            Reloading::<u8>::new(|| anyhow::bail!("no file"), |_| (ModelId([0; 32]), 0)).is_err()
        );
    }
}
