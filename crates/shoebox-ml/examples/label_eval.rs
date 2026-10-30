//! Calibrate label assignment (S1-13) on the eval set: for several softmax cut-offs, how often a
//! kept label (or one of its synonyms) is named in the photo's five captions, and how many labels
//! a photo gets. Captions miss things, so this understates precision: it is a floor.
//! `cargo run --release -p shoebox-ml --example label_eval`

use anyhow::Result;
use image::imageops::FilterType;
use shoebox_ml::ClipImage;
use shoebox_search::encoder::{ImageEncoder, Rgb};
use shoebox_search::labels::Labels;
use shoebox_search::models::CLIP_MODEL;
use shoebox_search::text::fold;
use std::path::PathBuf;

fn thumb(path: &std::path::Path) -> Result<Rgb> {
    let t = image::open(path)?
        .resize_to_fill(256, 256, FilterType::Triangle)
        .to_rgb8();
    let mut jpeg = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode_image(&t)?;
    let t = image::load_from_memory(&jpeg)?.to_rgb8();
    Ok(Rgb {
        width: t.width(),
        height: t.height(),
        data: t.into_raw(),
    })
}

fn named(captions: &str, term: &str) -> bool {
    let t = fold(term);
    let words: Vec<&str> = captions.split(|c: char| !c.is_alphanumeric()).collect();
    let tw: Vec<&str> = t.split(' ').collect();
    words.windows(tw.len()).any(|w| {
        w.iter().zip(&tw).all(|(a, b)| {
            a == b
                || *a == format!("{b}s")
                || *a == format!("{b}es")
                || b.strip_suffix('s') == Some(a)
        })
    })
}

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let eval = root.join("fixtures-out/search-eval");
    let cache = eval.join("image-vecs.bin");
    let items: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(eval.join("captions.json"))?)?;
    let vecs: Vec<Vec<f32>> = match std::fs::read(&cache) {
        Ok(b) if b.len() == items.len() * 512 * 4 => b
            .chunks(512 * 4)
            .map(|c| {
                c.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|x| f32::from_le_bytes(*x))
                    .collect()
            })
            .collect(),
        _ => {
            let enc = ClipImage::load(&root.join("models/build/clip-image-int8.onnx"), CLIP_MODEL)?;
            let mut out = vec![];
            let mut bytes = vec![];
            for it in &items {
                let v = enc
                    .encode_image(&thumb(
                        &eval.join("photos").join(it["file"].as_str().unwrap()),
                    )?)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                bytes.extend(v.iter().flat_map(|x| x.to_le_bytes()));
                out.push(v);
            }
            std::fs::write(&cache, bytes)?;
            out
        }
    };
    let labels = Labels::bundled();
    anyhow::ensure!(
        labels.model().is_some(),
        "labels.bin is missing or for another list"
    );
    let caps: Vec<String> = items
        .iter()
        .map(|it| {
            fold(
                &it["captions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(" . "),
            )
        })
        .collect();
    println!("| min share | labels kept per photo | photos with a label | named in a caption |\n|---|---|---|---|");
    for p in [0.03f32, 0.05, 0.08, 0.10, 0.15, 0.20, 0.30] {
        let (mut kept, mut hit, mut with) = (0usize, 0usize, 0usize);
        for (v, c) in vecs.iter().zip(&caps) {
            let ls = labels.assign(v, p);
            with += usize::from(!ls.is_empty());
            for (i, _) in &ls {
                kept += 1;
                let n = labels.name(*i).unwrap();
                if named(c, n) || labels.synonyms(*i).iter().any(|s| named(c, s)) {
                    hit += 1;
                }
            }
        }
        println!(
            "| {p:.2} | {:.2} | {:.1}% | {:.1}% |",
            kept as f64 / vecs.len() as f64,
            with as f64 * 100.0 / vecs.len() as f64,
            hit as f64 * 100.0 / kept.max(1) as f64
        );
    }
    // Labels by how often they fire, with how often a caption names them.
    let mut freq: std::collections::HashMap<u16, (usize, usize)> = Default::default();
    for (v, c) in vecs.iter().zip(&caps) {
        for (i, _) in labels.assign(v, 0.10) {
            let n = labels.name(i).unwrap();
            let ok = named(c, n) || labels.synonyms(i).iter().any(|s| named(c, s));
            let e = freq.entry(i).or_default();
            e.0 += 1;
            e.1 += usize::from(ok);
        }
    }
    let mut f: Vec<_> = freq.into_iter().collect();
    f.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    for (i, (n, ok)) in f.iter().take(60) {
        print!("{}:{n}/{ok}  ", labels.name(*i).unwrap());
    }
    println!();
    for (i, (v, it)) in vecs.iter().zip(&items).enumerate().skip(400).take(6) {
        let ls: Vec<String> = labels
            .assign(v, 0.10)
            .iter()
            .map(|(l, s)| format!("{} {:.2}", labels.name(*l).unwrap(), f32::from(*s) / 255.0))
            .collect();
        println!(
            "{i} {} | {} | {}",
            it["file"],
            ls.join(", "),
            it["captions"][0]
        );
    }
    Ok(())
}
