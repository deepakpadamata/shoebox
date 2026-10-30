//! Faces (S1-17) with OpenCV's model zoo: YuNet finds faces and five landmarks (MIT), SFace
//! describes an aligned face in 128 dimensions (Apache-2.0). Both run with tract.
//!
//! Inputs as OpenCV prepares them: YuNet takes BGR, 0–255, the image letterboxed into 640 × 640;
//! SFace takes the face aligned to its 112 × 112 template, RGB, 0–255.

use anyhow::{Context, Result};
use shoebox_search::encoder::{normalize, EncodeError, Rgb};
use shoebox_search::faces::{
    nms, similarity, warp, Detection, FaceDetector, FaceEmbedder, MIN_SCORE, TEMPLATE,
};
use std::path::Path;
use tract_onnx::prelude::*;

const SIDE: u32 = 640;
pub const FACE_DIMS: usize = 128;

type Plan = Arc<TypedRunnableModel>;

fn err(e: impl std::fmt::Display) -> EncodeError {
    EncodeError(e.to_string())
}

fn load(bytes: &[u8], input: InferenceFact) -> Result<Plan> {
    tract_onnx::onnx()
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, input)?
        .into_optimized()?
        .into_runnable()
}

pub struct YuNet {
    plan: Plan,
}

impl YuNet {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            plan: load(
                bytes,
                f32::fact([1, 3, SIDE as usize, SIDE as usize]).into(),
            )?,
        })
    }
}

impl FaceDetector for YuNet {
    fn detect(&self, image: &Rgb) -> Result<Vec<Detection>, EncodeError> {
        let img = image::RgbImage::from_raw(image.width, image.height, image.data.clone())
            .ok_or_else(|| err("image buffer does not match its size"))?;
        let scale = SIDE as f32 / image.width.max(image.height) as f32;
        let (w, h) = (
            ((image.width as f32 * scale).round() as u32).clamp(1, SIDE),
            ((image.height as f32 * scale).round() as u32).clamp(1, SIDE),
        );
        let small = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
        let s = SIDE as usize;
        let mut input = vec![0f32; 3 * s * s];
        for (x, y, p) in small.enumerate_pixels() {
            let i = y as usize * s + x as usize;
            // BGR, as OpenCV reads images.
            input[i] = f32::from(p.0[2]);
            input[s * s + i] = f32::from(p.0[1]);
            input[2 * s * s + i] = f32::from(p.0[0]);
        }
        let t = tract_ndarray::Array4::from_shape_vec((1, 3, s, s), input).map_err(err)?;
        let out = self.plan.run(tvec!(Tensor::from(t).into())).map_err(err)?;
        let view = |i: usize| -> Result<Vec<f32>, EncodeError> {
            Ok(out[i]
                .to_plain_array_view::<f32>()
                .map_err(err)?
                .iter()
                .copied()
                .collect())
        };
        let mut faces = vec![];
        for (k, stride) in [8u32, 16, 32].into_iter().enumerate() {
            let (cls, obj, bbox, kps) = (view(k)?, view(3 + k)?, view(6 + k)?, view(9 + k)?);
            let cols = SIDE / stride;
            let st = stride as f32;
            for idx in 0..cls.len() {
                let score = (cls[idx].clamp(0.0, 1.0) * obj[idx].clamp(0.0, 1.0)).sqrt();
                if score < MIN_SCORE {
                    continue;
                }
                let (r, c) = ((idx as u32 / cols) as f32, (idx as u32 % cols) as f32);
                let b = &bbox[idx * 4..idx * 4 + 4];
                let (cx, cy) = ((c + b[0]) * st, (r + b[1]) * st);
                let (bw, bh) = (b[2].exp() * st, b[3].exp() * st);
                let kp = &kps[idx * 10..idx * 10 + 10];
                let mut landmarks = [[0f32; 2]; 5];
                for (j, l) in landmarks.iter_mut().enumerate() {
                    *l = [
                        (kp[2 * j] + c) * st / scale,
                        (kp[2 * j + 1] + r) * st / scale,
                    ];
                }
                faces.push(Detection {
                    bbox: [
                        (cx - bw / 2.0) / scale,
                        (cy - bh / 2.0) / scale,
                        (cx + bw / 2.0) / scale,
                        (cy + bh / 2.0) / scale,
                    ],
                    landmarks,
                    score,
                });
            }
        }
        Ok(nms(faces, 0.3))
    }
}

pub struct SFace {
    plan: Plan,
}

impl SFace {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            plan: load(bytes, f32::fact([1, 3, 112, 112]).into())?,
        })
    }
}

impl FaceEmbedder for SFace {
    fn dims(&self) -> usize {
        FACE_DIMS
    }

    fn embed(&self, image: &Rgb, landmarks: &[[f32; 2]; 5]) -> Result<Vec<f32>, EncodeError> {
        let face = warp(image, similarity(landmarks, &TEMPLATE), 112);
        let n = 112 * 112;
        let mut input = vec![0f32; 3 * n];
        for (i, px) in face.data.as_chunks::<3>().0.iter().enumerate() {
            for c in 0..3 {
                input[c * n + i] = f32::from(px[c]);
            }
        }
        let t = tract_ndarray::Array4::from_shape_vec((1, 3, 112, 112), input).map_err(err)?;
        let out = self.plan.run(tvec!(Tensor::from(t).into())).map_err(err)?;
        let mut v: Vec<f32> = out[0]
            .to_plain_array_view::<f32>()
            .map_err(err)?
            .iter()
            .copied()
            .collect();
        normalize(&mut v);
        Ok(v)
    }
}
