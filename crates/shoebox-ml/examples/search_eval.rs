//! The search eval (S1-01): text → photo recall on captioned photos, with each photo indexed as a
//! device indexes it (its 256 px square thumbnail, JPEG quality 80), plus timings.
//!
//! ```text
//! python3 -I tools/search-eval/fetch.py 1000
//! cargo run --release -p shoebox-ml --example search_eval -- [models dir] [eval dir] [text model] [image model]
//! ```
//!
//! Recall@k: for each caption, whether its photo is among the k best of all photos.

use anyhow::{Context, Result};
use image::imageops::FilterType;
use shoebox_core::model::ModelId;
use shoebox_ml::{ClipImage, ClipText};
use shoebox_search::clip_tokenizer::ClipTokenizer;
use shoebox_search::encoder::{ImageEncoder, Rgb, TextEncoder};
use shoebox_search::shard::{sign_bits, Vec8};
use std::path::PathBuf;
use std::time::Instant;

fn thumb(path: &std::path::Path) -> Result<Rgb> {
    let img = image::open(path).with_context(|| path.display().to_string())?;
    let t = img.resize_to_fill(256, 256, FilterType::Triangle).to_rgb8();
    let mut jpeg = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode_image(&t)?;
    let t = image::load_from_memory(&jpeg)?.to_rgb8();
    Ok(Rgb {
        width: t.width(),
        height: t.height(),
        data: t.into_raw(),
    })
}

fn recall(ranks: &[usize], k: usize) -> f64 {
    ranks.iter().filter(|r| **r < k).count() as f64 / ranks.len() as f64 * 100.0
}

fn rank_of(scores: &[f32], target: usize) -> usize {
    let s = scores[target];
    scores.iter().filter(|x| **x > s).count()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let models = args
        .first()
        .map(PathBuf::from)
        .unwrap_or(root.join("models/src/clip"));
    let eval = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or(root.join("fixtures-out/search-eval"));
    let text_model = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or(models.join("textual/model.onnx"));
    let tok = ClipTokenizer::from_merges(include_str!("../../shoebox-search/data/clip-merges.txt"))
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let id = ModelId([0; 32]);
    let t0 = Instant::now();
    let text = ClipText::load(&text_model, tok, id)?;
    let text_load = t0.elapsed();
    let t0 = Instant::now();
    let image_model = args
        .get(3)
        .map(PathBuf::from)
        .unwrap_or(models.join("visual/model.onnx"));
    let image = ClipImage::load(&image_model, id)?;
    let image_load = t0.elapsed();

    let items: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(eval.join("captions.json"))?)?;
    let mut photos = vec![];
    let mut captions: Vec<(String, usize)> = vec![];
    for (i, it) in items.iter().enumerate() {
        photos.push(eval.join("photos").join(it["file"].as_str().unwrap()));
        for c in it["captions"].as_array().unwrap() {
            captions.push((c.as_str().unwrap().trim().to_string(), i));
        }
    }

    let t0 = Instant::now();
    let mut img_vecs = vec![];
    for p in &photos {
        img_vecs.push(
            image
                .encode_image(&thumb(p)?)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        );
    }
    let img_time = t0.elapsed();
    let t0 = Instant::now();
    let mut txt_vecs = vec![];
    for (c, _) in &captions {
        txt_vecs.push(text.encode_text(c).map_err(|e| anyhow::anyhow!("{e}"))?);
    }
    let txt_time = t0.elapsed();

    let q8: Vec<Vec<f32>> = img_vecs
        .iter()
        .map(|v| Vec8::quantize(v).to_f32())
        .collect();
    let bits: Vec<Vec<u8>> = img_vecs.iter().map(|v| sign_bits(v)).collect();
    let (mut r32, mut r8, mut r1) = (vec![], vec![], vec![]);
    for (t, (_, target)) in txt_vecs.iter().zip(&captions) {
        let dot = |v: &Vec<f32>| v.iter().zip(t).map(|(a, b)| a * b).sum::<f32>();
        r32.push(rank_of(
            &img_vecs.iter().map(dot).collect::<Vec<_>>(),
            *target,
        ));
        let s8: Vec<f32> = q8.iter().map(dot).collect();
        r8.push(rank_of(&s8, *target));
        // Binary first (Hamming against the query's sign bits), best 10% re-ranked with vec8.
        let qb = sign_bits(t);
        let mut ham: Vec<(u32, usize)> = bits
            .iter()
            .enumerate()
            .map(|(i, b)| {
                (
                    b.iter().zip(&qb).map(|(x, y)| (x ^ y).count_ones()).sum(),
                    i,
                )
            })
            .collect();
        ham.sort();
        let keep = (photos.len() / 10).max(10);
        let short: Vec<usize> = ham[..keep.min(ham.len())].iter().map(|(_, i)| *i).collect();
        r1.push(match short.iter().position(|i| i == target) {
            Some(_) => short.iter().filter(|i| s8[**i] > s8[*target]).count(),
            None => usize::MAX,
        });
    }
    // The relative cut (S1-12): how often the captioned photo survives, and how many are shown.
    let mut cuts = vec![];
    for margin in [0.02f32, 0.04, 0.06, 0.08, 0.10, 0.12] {
        let (mut kept_target, mut shown) = (0usize, 0usize);
        for (t, (_, target)) in txt_vecs.iter().zip(&captions) {
            let mut s: Vec<(usize, f32)> = q8
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.iter().zip(t).map(|(a, b)| a * b).sum::<f32>()))
                .collect();
            s.sort_by(|a, b| b.1.total_cmp(&a.1));
            let best = s[0].1;
            let kept: Vec<usize> = s
                .iter()
                .take_while(|x| x.1 >= best - margin)
                .map(|x| x.0)
                .collect();
            shown += kept.len();
            kept_target += usize::from(kept.contains(target));
        }
        cuts.push(format!(
            "| {margin:.2} | {:.1} | {:.1} |",
            kept_target as f64 * 100.0 / captions.len() as f64,
            shown as f64 / captions.len() as f64
        ));
    }
    println!("{} photos, {} captions", photos.len(), captions.len());
    println!(
        "load: text {:.2?}, image {:.2?}; image encoder {:.1} ms/photo; text encoder {:.1} ms/query",
        text_load,
        image_load,
        img_time.as_secs_f64() * 1000.0 / photos.len() as f64,
        txt_time.as_secs_f64() * 1000.0 / captions.len() as f64
    );
    println!("| | R@1 | R@5 | R@10 |");
    println!("|---|---|---|---|");
    for (name, r) in [
        ("fp32", &r32),
        ("vec8", &r8),
        ("vec1 → vec8 (top 10%)", &r1),
    ] {
        println!(
            "| {name} | {:.1} | {:.1} | {:.1} |",
            recall(r, 1),
            recall(r, 5),
            recall(r, 10)
        );
    }
    println!("\n| margin | captioned photo kept % | photos shown (mean) |\n|---|---|---|");
    for c in cuts {
        println!("{c}");
    }
    Ok(())
}
