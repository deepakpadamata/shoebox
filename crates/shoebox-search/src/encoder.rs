//! Models behind traits (epic S1 §4): `shoebox-search` never links an ML runtime. The node and
//! apps implement these with a real model (`shoebox-ml`); tests use [`TestEncoder`].
//!
//! Every embedding is L2-normalised, so a dot product is the cosine similarity.

use shoebox_core::model::ModelId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EncodeError {}

/// An 8-bit RGB image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Rgb {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 3) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }
}

/// Turns query text into the image–text space (every device).
pub trait TextEncoder: Send + Sync {
    fn model(&self) -> ModelId;
    fn dims(&self) -> usize;
    fn encode_text(&self, text: &str) -> Result<Vec<f32>, EncodeError>;
}

/// Turns a photo (its square thumbnail) into the same space (indexing devices).
pub trait ImageEncoder: Send + Sync {
    fn model(&self) -> ModelId;
    fn dims(&self) -> usize;
    fn encode_image(&self, image: &Rgb) -> Result<Vec<f32>, EncodeError>;
}

pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A deterministic stand-in for a real model, for tests on every device: each word is a fixed
/// pseudo-random direction; a text is the sum of its words; an image is the sum of the words
/// that describe its average colour and brightness ("red", "green", "blue", "dark", "bright",
/// "photo"). So a red photo is found by "red" and not by "blue".
#[derive(Clone, Copy, Debug)]
pub struct TestEncoder {
    pub dims: usize,
}

impl TestEncoder {
    pub const MODEL: ModelId = ModelId(*b"shoebox-test-encoder-v1.........");

    pub fn word(&self, w: &str) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.dims);
        let mut seed = *blake3::hash(crate::text::fold(w).as_bytes()).as_bytes();
        while out.len() < self.dims {
            for pair in seed.as_chunks::<2>().0 {
                let v = u16::from_le_bytes([pair[0], pair[1]]);
                out.push(f32::from(v) / 32_767.5 - 1.0);
            }
            seed = *blake3::hash(&seed).as_bytes();
        }
        out.truncate(self.dims);
        out
    }

    fn sum(&self, words: &[&str]) -> Vec<f32> {
        let mut v = vec![0f32; self.dims];
        for w in words {
            for (a, b) in v.iter_mut().zip(self.word(w)) {
                *a += b;
            }
        }
        normalize(&mut v);
        v
    }

    /// Words the test encoder sees in an image.
    pub fn describe(image: &Rgb) -> Vec<&'static str> {
        let n = (image.data.len() / 3).max(1) as u64;
        let mut sum = [0u64; 3];
        for px in image.data.as_chunks::<3>().0 {
            for c in 0..3 {
                sum[c] += u64::from(px[c]);
            }
        }
        let avg = sum.map(|s| s / n);
        let mut words = vec!["photo"];
        let (max, min) = (*avg.iter().max().unwrap(), *avg.iter().min().unwrap());
        if max - min > 40 {
            let i = avg.iter().position(|v| *v == max).unwrap();
            words.push(["red", "green", "blue"][i]);
        }
        let light = (avg[0] + avg[1] + avg[2]) / 3;
        if light < 70 {
            words.push("dark");
        } else if light > 190 {
            words.push("bright");
        }
        words
    }
}

impl TextEncoder for TestEncoder {
    fn model(&self) -> ModelId {
        Self::MODEL
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn encode_text(&self, text: &str) -> Result<Vec<f32>, EncodeError> {
        let words = crate::text::words(text);
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        Ok(self.sum(&refs))
    }
}

impl ImageEncoder for TestEncoder {
    fn model(&self) -> ModelId {
        Self::MODEL
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn encode_image(&self, image: &Rgb) -> Result<Vec<f32>, EncodeError> {
        Ok(self.sum(&Self::describe(image)))
    }
}
