//! Faces (S1-17): what an indexing device records about each face, and how a face is told
//! apart from the others in its photo across devices and model versions.
//!
//! A face is found on the preview by a [`FaceDetector`] (box, five landmarks, score), aligned
//! from its landmarks to the 112 × 112 template the embedder was trained on, and described by a
//! [`FaceEmbedder`] (128 dimensions, L2-normalised). Boxes and landmarks are stored as fractions
//! of the image in 1/65535 (`shard::Face`).
//!
//! A [`FaceKey`] is the box on a 1/64 grid: what people's corrections (`FaceLabel`, S1-18) are
//! keyed by. A label is matched to a detection by overlap (IoU > 0.5), so it survives another
//! device detecting the face slightly differently.

use crate::encoder::{EncodeError, Rgb};
use serde::{Deserialize, Serialize};

/// Faces smaller than this (pixels of the preview, the shorter side of the box) are skipped.
pub const MIN_FACE_PX: f32 = 40.0;
/// Detections below this score are skipped.
pub const MIN_SCORE: f32 = 0.8;
/// Two faces whose embeddings are at least this similar are the same person (SFace's
/// published cosine threshold).
pub const SAME_PERSON: f32 = 0.363;

/// A face found in an image, in pixels of that image.
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    /// left, top, right, bottom.
    pub bbox: [f32; 4],
    /// Right eye, left eye, nose tip, right and left mouth corner (as seen in the image: the
    /// person's right eye is on the image's left), as (x, y).
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

pub trait FaceDetector: Send + Sync {
    fn detect(&self, image: &Rgb) -> Result<Vec<Detection>, EncodeError>;
}

pub trait FaceEmbedder: Send + Sync {
    fn dims(&self) -> usize;
    /// The embedding of the face at `landmarks` in `image` (aligned by the embedder).
    fn embed(&self, image: &Rgb, landmarks: &[[f32; 2]; 5]) -> Result<Vec<f32>, EncodeError>;
}

/// Where a face is, on a 1/64 grid of the photo: (left, top, right, bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FaceKey(pub [u8; 4]);

impl FaceKey {
    /// From a box in 1/65535 of the image (as stored in shards).
    pub fn of(bbox: [u16; 4]) -> Self {
        Self(bbox.map(|v| ((u32::from(v) * 64 + 32_767) / 65_535).min(64) as u8))
    }

    /// The box as fractions of the image.
    pub fn rect(self) -> [f32; 4] {
        self.0.map(|v| f32::from(v) / 64.0)
    }

    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 8 {
            return None;
        }
        let mut out = [0u8; 4];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

/// Overlap of two boxes (left, top, right, bottom), 0 to 1.
pub fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let union = area(a) + area(b) - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// The detection in `boxes` (fractions of the image) a key refers to: the best overlap above 0.5.
pub fn match_key(key: FaceKey, boxes: &[[f32; 4]]) -> Option<usize> {
    let r = key.rect();
    boxes
        .iter()
        .enumerate()
        .map(|(i, b)| (i, iou(r, *b)))
        .filter(|(_, o)| *o > 0.5)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Where the five landmarks sit in the 112 × 112 face the embedder expects (ArcFace's template).
pub const TEMPLATE: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

/// The similarity transform (rotation, uniform scale, translation) taking `from` closest to `to`
/// in the least-squares sense (Umeyama), as `[a, -b, tx; b, a, ty]`: x' = a·x − b·y + tx,
/// y' = b·x + a·y + ty.
pub fn similarity(from: &[[f32; 2]; 5], to: &[[f32; 2]; 5]) -> [f32; 6] {
    let n = 5.0;
    let mean = |p: &[[f32; 2]; 5]| {
        let s = p
            .iter()
            .fold([0.0f32; 2], |a, q| [a[0] + q[0], a[1] + q[1]]);
        [s[0] / n, s[1] / n]
    };
    let (mf, mt) = (mean(from), mean(to));
    let (mut sxx, mut sxy, mut var) = (0.0f32, 0.0f32, 0.0f32);
    for (f, t) in from.iter().zip(to) {
        let (fx, fy) = (f[0] - mf[0], f[1] - mf[1]);
        let (tx, ty) = (t[0] - mt[0], t[1] - mt[1]);
        sxx += fx * tx + fy * ty;
        sxy += fx * ty - fy * tx;
        var += fx * fx + fy * fy;
    }
    let (a, b) = if var > 0.0 {
        (sxx / var, sxy / var)
    } else {
        (1.0, 0.0)
    };
    let tx = mt[0] - (a * mf[0] - b * mf[1]);
    let ty = mt[1] - (b * mf[0] + a * mf[1]);
    [a, -b, tx, b, a, ty]
}

/// Cut the aligned `size` × `size` face out of `image` with bilinear sampling: the inverse of
/// `m` maps each output pixel back into the image.
pub fn warp(image: &Rgb, m: [f32; 6], size: u32) -> Rgb {
    let (a, b, tx, ty) = (m[0], m[3], m[2], m[5]);
    let det = a * a + b * b;
    let mut out = Vec::with_capacity((size * size * 3) as usize);
    for y in 0..size {
        for x in 0..size {
            // Inverse of [a -b; b a] is [a b; -b a] / det.
            let (dx, dy) = (x as f32 - tx, y as f32 - ty);
            let sx = (a * dx + b * dy) / det;
            let sy = (-b * dx + a * dy) / det;
            out.extend_from_slice(&bilinear(image, sx, sy));
        }
    }
    Rgb {
        width: size,
        height: size,
        data: out,
    }
}

fn bilinear(img: &Rgb, x: f32, y: f32) -> [u8; 3] {
    if x < 0.0 || y < 0.0 || x > (img.width - 1) as f32 || y > (img.height - 1) as f32 {
        return [0, 0, 0];
    }
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width - 1), (y0 + 1).min(img.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |px, py| img.pixel(px, py).map(f32::from);
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut out = [0u8; 3];
    for i in 0..3 {
        let top = a[i] * (1.0 - fx) + b[i] * fx;
        let bot = c[i] * (1.0 - fx) + d[i] * fx;
        out[i] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Keep the best detections that do not overlap a better one by more than `threshold`.
pub fn nms(mut faces: Vec<Detection>, threshold: f32) -> Vec<Detection> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Detection> = vec![];
    for f in faces {
        if kept.iter().all(|k| iou(k.bbox, f.bbox) <= threshold) {
            kept.push(f);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_and_match_by_overlap() {
        let k = FaceKey::of([16_384, 8_192, 32_768, 40_960]);
        assert_eq!(k.0, [16, 8, 32, 40]);
        assert_eq!(FaceKey::from_hex(&k.to_hex()), Some(k));
        // Another device's detection, a little off: still the same face.
        let boxes = [[0.0, 0.0, 0.1, 0.1], [0.255, 0.13, 0.505, 0.63]];
        assert_eq!(match_key(k, &boxes), Some(1));
        assert_eq!(match_key(k, &[[0.6, 0.6, 0.9, 0.9]]), None);
    }

    #[test]
    fn similarity_recovers_a_known_transform() {
        // Rotate by 30°, scale by 2, shift by (5, -3).
        let (c, s) = (
            2.0 * 30f32.to_radians().cos(),
            2.0 * 30f32.to_radians().sin(),
        );
        let from = TEMPLATE;
        let to = from.map(|p| [c * p[0] - s * p[1] + 5.0, s * p[0] + c * p[1] - 3.0]);
        let m = similarity(&from, &to);
        for (got, want) in m.iter().zip([c, -s, 5.0, s, c, -3.0]) {
            assert!((got - want).abs() < 1e-3, "{m:?}");
        }
    }
}
