//! Check a text model runs under tract and how close two variants are (cosine), with timings.
//! `cargo run --release -p shoebox-ml --example clip_check -- <a.onnx> <b.onnx>`

use shoebox_core::model::ModelId;
use shoebox_ml::ClipText;
use shoebox_search::clip_tokenizer::ClipTokenizer;
use shoebox_search::encoder::{dot, TextEncoder};
use std::time::Instant;

fn rss_mb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    out / 1024
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tok = || {
        ClipTokenizer::from_merges(include_str!("../../shoebox-search/data/clip-merges.txt"))
            .unwrap()
    };
    let _ = tok();
    println!("baseline rss {} MB", rss_mb());
    let mut models = vec![];
    for p in &args {
        let t = Instant::now();
        models.push(ClipText::load(p.as_ref(), tok(), ModelId([0; 32]))?);
        println!("{p}: loaded in {:.2?}, rss {} MB", t.elapsed(), rss_mb());
    }
    for text in [
        "a dog on a beach",
        "birthday cake with candles",
        "receipt from a restaurant",
    ] {
        let mut vs = vec![];
        for (m, p) in models.iter().zip(&args) {
            let t = Instant::now();
            let n = 20;
            let mut v = vec![];
            for _ in 0..n {
                v = m.encode_text(text).unwrap();
            }
            println!(
                "{text:?} {p}: {:.1} ms, first {:?}",
                t.elapsed().as_secs_f64() * 1000.0 / n as f64,
                &v[..4]
            );
            vs.push(v);
        }
        println!("  rss {} MB", rss_mb());
        if vs.len() == 2 {
            println!("  cosine between them: {:.4}", dot(&vs[0], &vs[1]));
        }
    }
    Ok(())
}
