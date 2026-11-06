//! Text in photos (S2-03) with PaddleOCR's PP-OCRv5 mobile models on tract: one engine for Android,
//! Windows, Linux and the keeper (the Mac keeps Apple Vision). Open models (Apache-2.0), nothing
//! leaves the device.
//!
//! The pre- and post-processing follow PaddleOCR's own (`inference.yml` of the two models):
//! - **Detection** (DB): the photo with its long side set to 960 and both sides a multiple of 32; the
//!   probability map above 0.3 is cut into regions, a region scoring under 0.6 is dropped, and the
//!   rest are turned into the smallest rotated rectangle and grown by 1.5 (the DB "unclip").
//! - **Recognition** (CTC, greedy): each rectangle is straightened into an upright crop, turned a
//!   quarter if it is much taller than wide (vertical text), scaled to 48 px high and read; a line
//!   scoring under 0.5 is dropped.
//!
//! Not done: PaddleOCR's optional text-line orientation model (text upside down is not read).
//!
//! The model files (`ocr-det-ppocrv5-mobile.onnx`, `ocr-rec-ppocrv5-latin.onnx`, and the
//! recogniser's dictionary, one character a line) come from `cargo xtask models`. Plans are built
//! once per input shape and kept, so later photos of the same shape start at once.

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::RgbImage;
use shoebox_search::encoder::EncodeError;
use shoebox_search::models::OCR_DET;
use shoebox_search::ocr::Ocr;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use tract_onnx::prelude::*;

type Plan = Arc<TypedRunnableModel>;

/// The longest side of a photo given to the reader (the recogniser reads crops of it at this
/// size, so small print stays legible); the detector sees it at [`DET_LONG`].
const MAX_EDGE: u32 = 2048;
const DET_LONG: f32 = 960.0;
const DET_THRESH: f32 = 0.3;
const DET_BOX_SCORE: f32 = 0.6;
const DET_UNCLIP: f32 = 1.5;
const DET_MIN_SIDE: f32 = 3.0;
const REC_H: usize = 48;
const REC_MIN_W: usize = 320;
const REC_MAX_W: usize = 3200;
/// A line read with a mean character score under this is dropped (PaddleOCR's `drop_score`).
const DROP_SCORE: f32 = 0.5;
/// A crop at least this much taller than wide is vertical text and is turned a quarter.
const VERTICAL: f32 = 1.5;
/// Plans kept, so a library of odd shapes cannot grow memory without limit.
const MAX_DET_PLANS: usize = 16;
const MAX_REC_PLANS: usize = 40;

fn err(e: impl std::fmt::Display) -> EncodeError {
    EncodeError(e.to_string())
}

fn plan(bytes: &[u8], shape: [usize; 4]) -> Result<Plan> {
    // The exported graphs carry shape annotations tract cannot unify with its own.
    tract_onnx::onnx()
        .with_ignore_value_info(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, InferenceFact::dt_shape(f32::datum_type(), shape))?
        .into_optimized()?
        .into_runnable()
}

/// PP-OCRv5 mobile detection and Latin recognition.
pub struct PpOcr {
    det: Vec<u8>,
    rec: Vec<u8>,
    /// Index 0 of the recogniser's output is the blank, then these, then the space character.
    dict: Vec<String>,
    det_plans: Mutex<HashMap<(usize, usize), Plan>>,
    rec_plans: Mutex<HashMap<usize, Plan>>,
}

impl PpOcr {
    pub fn load(det: &Path, rec: &Path, dict: &Path) -> Result<Self> {
        let read = |p: &Path| std::fs::read(p).with_context(|| p.display().to_string());
        Self::from_bytes(&read(det)?, &read(rec)?, &String::from_utf8(read(dict)?)?)
    }

    /// The reader from the model files found (and checked against the compiled-in hashes) in
    /// `dirs`; `None` when any of the three is not there.
    pub fn locate(dirs: &[std::path::PathBuf]) -> Result<Option<Self>> {
        use shoebox_search::models::{OCR_DICT, OCR_REC};
        let find = |f| crate::files::locate(f, dirs);
        let (Some(det), Some(rec), Some(dict)) = (find(&OCR_DET), find(&OCR_REC), find(&OCR_DICT))
        else {
            return Ok(None);
        };
        Self::load(&det, &rec, &dict).map(Some)
    }

    /// From the model files' contents (a phone may hold them as blobs from a peer or the release).
    pub fn from_bytes(det: &[u8], rec: &[u8], dict: &str) -> Result<Self> {
        let mut dict: Vec<String> = dict.lines().map(str::to_string).collect();
        if dict.is_empty() {
            bail!("the recogniser's dictionary is empty");
        }
        dict.push(" ".into());
        let me = Self {
            det: det.to_vec(),
            rec: rec.to_vec(),
            dict,
            det_plans: Mutex::new(HashMap::new()),
            rec_plans: Mutex::new(HashMap::new()),
        };
        // Both graphs must load: build the smallest plans now so a bad file fails here.
        me.det_plan(32, 32).context("the detection model")?;
        me.rec_plan(REC_MIN_W).context("the recognition model")?;
        Ok(me)
    }

    fn det_plan(&self, h: usize, w: usize) -> Result<Plan> {
        let mut plans = self.det_plans.lock().unwrap();
        if let Some(p) = plans.get(&(h, w)) {
            return Ok(p.clone());
        }
        let p = plan(&self.det, [1, 3, h, w])?;
        if plans.len() >= MAX_DET_PLANS {
            plans.clear();
        }
        plans.insert((h, w), p.clone());
        Ok(p)
    }

    fn rec_plan(&self, w: usize) -> Result<Plan> {
        let mut plans = self.rec_plans.lock().unwrap();
        if let Some(p) = plans.get(&w) {
            return Ok(p.clone());
        }
        let p = plan(&self.rec, [1, 3, REC_H, w])?;
        if plans.len() >= MAX_REC_PLANS {
            plans.clear();
        }
        plans.insert(w, p.clone());
        Ok(p)
    }

    /// Text regions of an upright photo, in reading order.
    fn detect(&self, img: &RgbImage) -> Result<Vec<Quad>> {
        let (ow, oh) = (img.width() as f32, img.height() as f32);
        let ratio = DET_LONG / ow.max(oh);
        let round32 = |v: f32| (((v * ratio) / 32.0).round() as usize).max(1) * 32;
        let (h, w) = (round32(oh), round32(ow));
        let small = image::imageops::resize(img, w as u32, h as u32, FilterType::Triangle);
        // PaddleOCR decodes with OpenCV (BGR) and applies the mean and std in that order.
        let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
        let mut input = vec![0f32; 3 * h * w];
        for (x, y, p) in small.enumerate_pixels() {
            let i = y as usize * w + x as usize;
            for (c, src) in [2usize, 1, 0].into_iter().enumerate() {
                input[c * h * w + i] = (f32::from(p.0[src]) / 255.0 - mean[c]) / std[c];
            }
        }
        let t = tract_ndarray::Array4::from_shape_vec((1, 3, h, w), input)?;
        let out = self.det_plan(h, w)?.run(tvec!(Tensor::from(t).into()))?;
        let prob: Vec<f32> = out[0]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect();
        if prob.len() != h * w {
            bail!("the detector answered {} values for {h}x{w}", prob.len());
        }
        let (sx, sy) = (ow / w as f32, oh / h as f32);
        let mut quads: Vec<Quad> = regions(&prob, w, h)
            .into_iter()
            .filter_map(|r| r.quad(sx, sy, ow, oh))
            .collect();
        // Reading order: top to bottom, left to right within a row (PaddleOCR's `sort_boxes`).
        quads.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        for i in 0..quads.len().saturating_sub(1) {
            for j in (0..=i).rev() {
                if (quads[j + 1].top() - quads[j].top()).abs() < 10.0
                    && quads[j + 1].left() < quads[j].left()
                {
                    quads.swap(j, j + 1);
                } else {
                    break;
                }
            }
        }
        Ok(quads)
    }

    /// The text in one region and the mean score of its characters.
    fn recognise(&self, img: &RgbImage, q: &Quad) -> Result<(String, f32)> {
        let mut crop = q.crop(img);
        if crop.height() as f32 / crop.width() as f32 >= VERTICAL {
            crop = image::imageops::rotate270(&crop);
        }
        let ratio = crop.width() as f32 / crop.height() as f32;
        let rw = ((REC_H as f32 * ratio).ceil() as usize).clamp(1, REC_MAX_W);
        // Width of the input: at least 320, then a multiple of 32 so plans are shared.
        let w = rw.max(REC_MIN_W).div_ceil(32) * 32;
        let small = image::imageops::resize(&crop, rw as u32, REC_H as u32, FilterType::Triangle);
        let mut input = vec![0f32; 3 * REC_H * w];
        for (x, y, p) in small.enumerate_pixels() {
            let i = y as usize * w + x as usize;
            for (c, src) in [2usize, 1, 0].into_iter().enumerate() {
                input[c * REC_H * w + i] = (f32::from(p.0[src]) / 255.0 - 0.5) / 0.5;
            }
        }
        let t = tract_ndarray::Array4::from_shape_vec((1, 3, REC_H, w), input)?;
        let out = self.rec_plan(w)?.run(tvec!(Tensor::from(t).into()))?;
        let view = out[0].to_plain_array_view::<f32>()?;
        let shape = view.shape().to_vec();
        if shape.len() != 3 {
            bail!("the recogniser answered a tensor of shape {shape:?}");
        }
        let (steps, classes) = (shape[1], shape[2]);
        let flat: Vec<f32> = view.iter().copied().collect();
        let (mut text, mut last, mut score, mut n) = (String::new(), 0usize, 0f32, 0usize);
        for s in 0..steps {
            let row = &flat[s * classes..(s + 1) * classes];
            let (best, p) = row
                .iter()
                .copied()
                .enumerate()
                .fold((0, f32::MIN), |m, (i, v)| if v > m.1 { (i, v) } else { m });
            if best != 0 && best != last {
                match self.dict.get(best - 1) {
                    Some(c) => text.push_str(c),
                    None => text.push(' '),
                }
                score += p;
                n += 1;
            }
            last = best;
        }
        Ok((text, if n == 0 { 0.0 } else { score / n as f32 }))
    }

    /// The lines of text in an upright photo, top to bottom.
    pub fn read_image(&self, img: &RgbImage) -> Result<Vec<String>> {
        let mut lines = vec![];
        for q in self.detect(img)? {
            let (text, score) = self.recognise(img, &q)?;
            let text = text.trim();
            if score >= DROP_SCORE && !text.is_empty() {
                lines.push(text.to_string());
            }
        }
        Ok(lines)
    }
}

impl Ocr for PpOcr {
    fn read_text(&self, bytes: &[u8]) -> Result<String, EncodeError> {
        let img = shoebox_media::decode_upright(bytes, MAX_EDGE).map_err(err)?;
        let lines = self.read_image(&img.to_rgb8()).map_err(err)?;
        Ok(lines.join("\n"))
    }
}

/// A text region: the corners of a rotated rectangle in the photo's pixels, clockwise from the
/// top left of the text's own frame (so `0 → 1` runs along the text).
#[derive(Debug, Clone, Copy)]
struct Quad {
    c: [(f32, f32); 4],
}

impl Quad {
    fn top(&self) -> f32 {
        self.c.iter().map(|p| p.1).fold(f32::MAX, f32::min)
    }

    fn left(&self) -> f32 {
        self.c.iter().map(|p| p.0).fold(f32::MAX, f32::min)
    }

    /// The region straightened into an upright rectangle at the photo's own resolution.
    fn crop(&self, img: &RgbImage) -> RgbImage {
        let dist =
            |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        let cw = dist(self.c[0], self.c[1]).round().clamp(1.0, 8192.0) as u32;
        let ch = dist(self.c[0], self.c[3]).round().clamp(1.0, 8192.0) as u32;
        let (tl, tr, bl) = (self.c[0], self.c[1], self.c[3]);
        let mut out = RgbImage::new(cw, ch);
        for v in 0..ch {
            let fy = (v as f32 + 0.5) / ch as f32;
            for u in 0..cw {
                let fx = (u as f32 + 0.5) / cw as f32;
                let x = tl.0 + fx * (tr.0 - tl.0) + fy * (bl.0 - tl.0);
                let y = tl.1 + fx * (tr.1 - tl.1) + fy * (bl.1 - tl.1);
                out.put_pixel(u, v, bilinear(img, x - 0.5, y - 0.5));
            }
        }
        out
    }
}

fn bilinear(img: &RgbImage, x: f32, y: f32) -> image::Rgb<u8> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: i32, yi: i32| img.get_pixel(xi.clamp(0, w - 1) as u32, yi.clamp(0, h - 1) as u32);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (
        at(x0, y0),
        at(x0 + 1, y0),
        at(x0, y0 + 1),
        at(x0 + 1, y0 + 1),
    );
    let mut px = [0u8; 3];
    for (i, p) in px.iter_mut().enumerate() {
        let top = f32::from(a.0[i]) * (1.0 - fx) + f32::from(b.0[i]) * fx;
        let bottom = f32::from(c.0[i]) * (1.0 - fx) + f32::from(d.0[i]) * fx;
        *p = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    image::Rgb(px)
}

/// A connected region of the probability map above the threshold.
struct Region {
    /// Per row, the leftmost and rightmost pixel (the hull of the region is the hull of these).
    rows: HashMap<usize, (usize, usize)>,
    sum: f32,
    count: usize,
}

impl Region {
    /// The text rectangle this region stands for, in the photo's pixels; `None` if it is too
    /// small or scores too low.
    fn quad(&self, sx: f32, sy: f32, ow: f32, oh: f32) -> Option<Quad> {
        if self.sum / self.count as f32 <= DET_BOX_SCORE {
            return None;
        }
        let mut pts: Vec<(f32, f32)> = vec![];
        for (&y, &(a, b)) in &self.rows {
            pts.push((a as f32, y as f32));
            if b != a {
                pts.push((b as f32, y as f32));
            }
        }
        let hull = convex_hull(pts);
        let (centre, u, (w, h)) = min_area_rect(&hull);
        // Pixels are cells: the region spans one more than the distance between centres.
        let (w, h) = (w + 1.0, h + 1.0);
        if w.min(h) < DET_MIN_SIDE {
            return None;
        }
        // The map holds the shrunk text kernel; unclip restores the text's extent.
        let d = w * h * DET_UNCLIP / (2.0 * (w + h));
        let (w, h) = (w + 2.0 * d, h + 2.0 * d);
        if w.min(h) < DET_MIN_SIDE + 2.0 {
            return None;
        }
        let v = (-u.1, u.0);
        let corner = |a: f32, b: f32| {
            (
                ((centre.0 + 0.5 + a * u.0 * w / 2.0 + b * v.0 * h / 2.0) * sx).clamp(0.0, ow),
                ((centre.1 + 0.5 + a * u.1 * w / 2.0 + b * v.1 * h / 2.0) * sy).clamp(0.0, oh),
            )
        };
        Some(Quad {
            c: [
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ],
        })
    }
}

/// 8-connected regions of the probability map above the threshold.
fn regions(prob: &[f32], w: usize, h: usize) -> Vec<Region> {
    let mut seen = vec![false; w * h];
    let mut out = vec![];
    let mut stack = vec![];
    for start in 0..w * h {
        if seen[start] || prob[start] <= DET_THRESH {
            continue;
        }
        let mut r = Region {
            rows: HashMap::new(),
            sum: 0.0,
            count: 0,
        };
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let e = r.rows.entry(y).or_insert((x, x));
            e.0 = e.0.min(x);
            e.1 = e.1.max(x);
            r.sum += prob[i];
            r.count += 1;
            for dy in [-1i32, 0, 1] {
                for dx in [-1i32, 0, 1] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if !seen[j] && prob[j] > DET_THRESH {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        out.push(r);
    }
    out
}

/// Andrew's monotone chain.
fn convex_hull(mut pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut hull: Vec<(f32, f32)> = vec![];
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &(f32, f32)>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in iter {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// The smallest rectangle around a convex polygon: its centre, the unit vector along its longer
/// side turned into the half-plane pointing right (so text reads left to right when it is
/// within 45° of level), and the lengths along that vector and across it.
type Rect = ((f32, f32), (f32, f32), (f32, f32));

fn min_area_rect(hull: &[(f32, f32)]) -> Rect {
    if hull.len() < 3 {
        let (min_x, max_x) = hull
            .iter()
            .fold((f32::MAX, f32::MIN), |m, p| (m.0.min(p.0), m.1.max(p.0)));
        let (min_y, max_y) = hull
            .iter()
            .fold((f32::MAX, f32::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
        return (
            ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0),
            (1.0, 0.0),
            (max_x - min_x, max_y - min_y),
        );
    }
    let mut best: Option<(f32, Rect)> = None;
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if len == 0.0 {
            continue;
        }
        let u = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let v = (-u.1, u.0);
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in hull {
            let (pu, pv) = (p.0 * u.0 + p.1 * u.1, p.0 * v.0 + p.1 * v.1);
            u0 = u0.min(pu);
            u1 = u1.max(pu);
            v0 = v0.min(pv);
            v1 = v1.max(pv);
        }
        let area = (u1 - u0) * (v1 - v0);
        if best.is_none_or(|(a, _)| area < a) {
            let (cu, cv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            let centre = (cu * u.0 + cv * v.0, cu * u.1 + cv * v.1);
            best = Some((area, (centre, u, (u1 - u0, v1 - v0))));
        }
    }
    let (_, (centre, mut u, (mut w, mut h))) =
        best.unwrap_or((0.0, (hull[0], (1.0, 0.0), (0.0, 0.0))));
    // Text runs along the axis within 45° of level: make that one the width.
    if u.0.abs() < u.1.abs() {
        u = (-u.1, u.0);
        std::mem::swap(&mut w, &mut h);
    }
    if u.0 < 0.0 {
        u = (-u.0, -u.1);
    }
    (centre, u, (w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rectangle_around_a_tilted_bar_is_found() {
        // A 40 x 10 bar turned by 20 degrees.
        let (s, c) = 20f32.to_radians().sin_cos();
        let mut pts = vec![];
        for (x, y) in [(0.0, 0.0), (40.0, 0.0), (40.0, 10.0), (0.0, 10.0)] {
            pts.push((10.0 + x * c - y * s, 10.0 + x * s + y * c));
        }
        let (centre, u, (w, h)) = min_area_rect(&convex_hull(pts));
        assert!(
            (w - 40.0).abs() < 0.01 && (h - 10.0).abs() < 0.01,
            "{w} x {h}"
        );
        assert!((u.0 - c).abs() < 0.001 && (u.1 - s).abs() < 0.001, "{u:?}");
        assert!((centre.0 - (10.0 + 20.0 * c - 5.0 * s)).abs() < 0.01);
    }

    #[test]
    fn a_tall_bar_is_turned_so_its_width_is_the_short_side() {
        let pts = vec![(0.0, 0.0), (6.0, 0.0), (6.0, 50.0), (0.0, 50.0)];
        let (_, u, (w, h)) = min_area_rect(&convex_hull(pts));
        // The long side is not the one within 45 degrees of level, so the width is the 6.
        assert!(
            (w - 6.0).abs() < 0.01 && (h - 50.0).abs() < 0.01,
            "{w} x {h} {u:?}"
        );
        assert!(u.0 > 0.99);
    }

    #[test]
    fn a_crop_of_a_tilted_region_is_upright() {
        // A white image with a black 30 x 8 bar turned by 15 degrees; its quad crops to the bar.
        let mut img = RgbImage::from_pixel(120, 80, image::Rgb([255, 255, 255]));
        let (s, c) = 15f32.to_radians().sin_cos();
        for v in 0..8 {
            for u in 0..30 {
                let (x, y) = (
                    30.0 + u as f32 * c - v as f32 * s,
                    30.0 + u as f32 * s + v as f32 * c,
                );
                img.put_pixel(x as u32, y as u32, image::Rgb([0, 0, 0]));
            }
        }
        let q = Quad {
            c: [
                (30.0, 30.0),
                (30.0 + 30.0 * c, 30.0 + 30.0 * s),
                (30.0 + 30.0 * c - 8.0 * s, 30.0 + 30.0 * s + 8.0 * c),
                (30.0 - 8.0 * s, 30.0 + 8.0 * c),
            ],
        };
        let crop = q.crop(&img);
        assert_eq!((crop.width(), crop.height()), (30, 8));
        let dark = crop.pixels().filter(|p| p.0[0] < 128).count();
        assert!(dark > 200, "only {dark} of 240 pixels are dark");
    }
}
