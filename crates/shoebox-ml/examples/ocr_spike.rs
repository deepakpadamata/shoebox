//! S2-03 spike: read the text in photos with PP-OCRv5 (mobile detection, Latin recognition) on tract.
//!
//! ```text
//! cargo run --release -p shoebox-ml --example ocr_spike -- <det.onnx> <rec.onnx> <dict.txt> <image or folder>...
//! ```
//!
//! Prints one JSON object per image: the lines read (text, score, box in the image's pixels, reading
//! order) and the time each stage took. It follows PaddleOCR's own pre- and post-processing for the
//! mobile models (`inference.yml`): detection on the image with its long side set to 960 and both sides
//! a multiple of 32, DB post-processing (threshold 0.3, box score 0.6, unclip 1.5); recognition of each
//! box at 48 px high, at least 320 wide, and greedy CTC. Boxes are axis-aligned (PaddleOCR uses rotated
//! rectangles), and vertical text is not turned: both are things the engine proper would add.

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tract_onnx::prelude::*;

type Plan = Arc<TypedRunnableModel>;
/// Boxes as `[x0, y0, x1, y1]` in the image's pixels, and the (height, width) the detector ran at.
type Detected = (Vec<[f32; 4]>, (usize, usize));

const DET_LONG: f32 = 960.0;
const DET_THRESH: f32 = 0.3;
const DET_BOX_SCORE: f32 = 0.6;
const DET_UNCLIP: f32 = 1.5;
const DET_MIN_SIDE: f32 = 3.0;
const REC_H: usize = 48;
const REC_MIN_W: usize = 320;
const REC_MAX_W: usize = 3200;

fn plan(bytes: &[u8], shape: [usize; 4]) -> Result<Plan> {
    tract_onnx::onnx()
        .with_ignore_value_info(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, InferenceFact::dt_shape(f32::datum_type(), shape))?
        .into_optimized()?
        .into_runnable()
}

struct Ocr {
    det_bytes: Vec<u8>,
    rec_bytes: Vec<u8>,
    dict: Vec<String>,
    det_plans: HashMap<(usize, usize), Plan>,
    rec_plans: HashMap<usize, Plan>,
    /// Time spent building plans (the first use of each input shape), kept apart from running.
    plan_ms: f64,
}

#[derive(Debug, Clone)]
struct Line {
    text: String,
    score: f32,
    bbox: [f32; 4],
}

impl Ocr {
    fn det_plan(&mut self, h: usize, w: usize) -> Result<Plan> {
        if let Some(p) = self.det_plans.get(&(h, w)) {
            return Ok(p.clone());
        }
        let t = Instant::now();
        let p = plan(&self.det_bytes, [1, 3, h, w])?;
        self.plan_ms += t.elapsed().as_secs_f64() * 1e3;
        self.det_plans.insert((h, w), p.clone());
        Ok(p)
    }

    fn rec_plan(&mut self, w: usize) -> Result<Plan> {
        if let Some(p) = self.rec_plans.get(&w) {
            return Ok(p.clone());
        }
        let t = Instant::now();
        let p = plan(&self.rec_bytes, [1, 3, REC_H, w])?;
        self.plan_ms += t.elapsed().as_secs_f64() * 1e3;
        self.rec_plans.insert(w, p.clone());
        Ok(p)
    }

    /// Text regions as boxes in the image's pixels, in reading order.
    fn detect(&mut self, img: &image::RgbImage) -> Result<Detected> {
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
        let plan = self.det_plan(h, w)?;
        let t = tract_ndarray::Array4::from_shape_vec((1, 3, h, w), input)?;
        let out = plan.run(tvec!(Tensor::from(t).into()))?;
        let prob: Vec<f32> = out[0]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect();
        if prob.len() != h * w {
            bail!("the detector answered {} values for {h}x{w}", prob.len());
        }
        let (sx, sy) = (ow / w as f32, oh / h as f32);
        let mut boxes: Vec<[f32; 4]> = components(&prob, w, h)
            .into_iter()
            .filter_map(|c| {
                let (bw, bh) = ((c.x1 - c.x0 + 1) as f32, (c.y1 - c.y0 + 1) as f32);
                if bw.min(bh) < DET_MIN_SIDE || c.sum / c.count as f32 <= DET_BOX_SCORE {
                    return None;
                }
                // The map holds the shrunk text kernel; unclip restores the text's extent.
                let d = bw * bh * DET_UNCLIP / (2.0 * (bw + bh));
                let (x0, y0) = (c.x0 as f32 - d, c.y0 as f32 - d);
                let (x1, y1) = (c.x1 as f32 + 1.0 + d, c.y1 as f32 + 1.0 + d);
                if (x1 - x0).min(y1 - y0) < DET_MIN_SIDE + 2.0 {
                    return None;
                }
                Some([
                    (x0 * sx).clamp(0.0, ow - 1.0),
                    (y0 * sy).clamp(0.0, oh - 1.0),
                    (x1 * sx).clamp(1.0, ow),
                    (y1 * sy).clamp(1.0, oh),
                ])
            })
            .collect();
        // Reading order: top to bottom, left to right within a row (PaddleOCR's `sort_boxes`).
        boxes.sort_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])));
        for i in 0..boxes.len().saturating_sub(1) {
            for j in (0..=i).rev() {
                if (boxes[j + 1][1] - boxes[j][1]).abs() < 10.0 && boxes[j + 1][0] < boxes[j][0] {
                    boxes.swap(j, j + 1);
                } else {
                    break;
                }
            }
        }
        Ok((boxes, (h, w)))
    }

    fn recognise(&mut self, img: &image::RgbImage, b: [f32; 4]) -> Result<Line> {
        let (x0, y0) = (b[0].floor() as u32, b[1].floor() as u32);
        let (x1, y1) = (
            (b[2].ceil() as u32).min(img.width()),
            (b[3].ceil() as u32).min(img.height()),
        );
        let crop =
            image::imageops::crop_imm(img, x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)).to_image();
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
        let plan = self.rec_plan(w)?;
        let t = tract_ndarray::Array4::from_shape_vec((1, 3, REC_H, w), input)?;
        let out = plan.run(tvec!(Tensor::from(t).into()))?;
        let view = out[0].to_plain_array_view::<f32>()?;
        let shape = view.shape().to_vec();
        let (steps, classes) = (shape[1], shape[2]);
        let flat: Vec<f32> = view.iter().copied().collect();
        // Index 0 is the blank, then the dictionary, then the space character.
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
        Ok(Line {
            text,
            score: if n == 0 { 0.0 } else { score / n as f32 },
            bbox: b,
        })
    }

    fn read(&mut self, img: &image::RgbImage) -> Result<(Vec<Line>, Timing)> {
        let p0 = self.plan_ms;
        let t0 = Instant::now();
        let (boxes, shape) = self.detect(img)?;
        let det_plan = self.plan_ms - p0;
        let det_ms = t0.elapsed().as_secs_f64() * 1e3 - det_plan;
        let p1 = self.plan_ms;
        let t1 = Instant::now();
        let mut lines = vec![];
        for b in &boxes {
            lines.push(self.recognise(img, *b)?);
        }
        let rec_plan = self.plan_ms - p1;
        let rec_ms = t1.elapsed().as_secs_f64() * 1e3 - rec_plan;
        Ok((
            lines,
            Timing {
                det_ms,
                rec_ms,
                plan_ms: det_plan + rec_plan,
                shape,
            },
        ))
    }
}

struct Timing {
    /// Running the detector, without building its plan.
    det_ms: f64,
    /// Running the recogniser on every box, without building its plans.
    rec_ms: f64,
    /// Building plans for input shapes not seen before (shared by later images).
    plan_ms: f64,
    shape: (usize, usize),
}

struct Comp {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    sum: f32,
    count: usize,
}

/// 8-connected regions of the probability map above the threshold.
fn components(prob: &[f32], w: usize, h: usize) -> Vec<Comp> {
    let mut seen = vec![false; w * h];
    let mut out = vec![];
    let mut stack = vec![];
    for start in 0..w * h {
        if seen[start] || prob[start] <= DET_THRESH {
            continue;
        }
        let (sx, sy) = (start % w, start / w);
        let mut c = Comp {
            x0: sx,
            y0: sy,
            x1: sx,
            y1: sy,
            sum: 0.0,
            count: 0,
        };
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            c.x0 = c.x0.min(x);
            c.x1 = c.x1.max(x);
            c.y0 = c.y0.min(y);
            c.y1 = c.y1.max(y);
            c.sum += prob[i];
            c.count += 1;
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
        out.push(c);
    }
    out
}

fn images(args: &[String]) -> Result<Vec<PathBuf>> {
    let mut out = vec![];
    for a in args {
        let p = Path::new(a);
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(p)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        ["jpg", "jpeg", "png"].contains(&e.to_lowercase().as_str())
                    })
                })
                .collect();
            v.sort();
            out.extend(v);
        } else {
            out.push(p.to_path_buf());
        }
    }
    Ok(out)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        bail!("usage: ocr_spike <det.onnx> <rec.onnx> <dict.txt> <image or folder>...");
    }
    let t = Instant::now();
    let mut dict: Vec<String> = std::fs::read_to_string(&args[2])?
        .lines()
        .map(str::to_string)
        .collect();
    dict.push(" ".into());
    let mut ocr = Ocr {
        det_bytes: std::fs::read(&args[0]).context("detector")?,
        rec_bytes: std::fs::read(&args[1]).context("recogniser")?,
        dict,
        det_plans: HashMap::new(),
        rec_plans: HashMap::new(),
        plan_ms: 0.0,
    };
    eprintln!("models read in {:.0?}", t.elapsed());
    for path in images(&args[3..])? {
        let img = image::open(&path)?.to_rgb8();
        let t = Instant::now();
        let (lines, timing) = ocr.read(&img)?;
        let total = t.elapsed().as_secs_f64() * 1e3;
        let lines: Vec<serde_json::Value> = lines
            .iter()
            .map(|l| {
                serde_json::json!({
                    "text": l.text,
                    "score": (l.score * 1000.0).round() / 1000.0,
                    "box": l.bbox.map(|v| v.round()),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "file": path.file_name().unwrap().to_string_lossy(),
                "size": [img.width(), img.height()],
                "det_shape": [timing.shape.0, timing.shape.1],
                "det_ms": timing.det_ms.round(),
                "rec_ms": timing.rec_ms.round(),
                "plan_ms": timing.plan_ms.round(),
                "total_ms": total.round(),
                "lines": lines,
            })
        );
    }
    Ok(())
}
