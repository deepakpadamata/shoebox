//! S2-03: read the text in images with the engine the apps use (`PpOcr`), one JSON object per
//! image: the lines read and the time it took. `scripts/ocr_spike_eval.py compare` scores the output.
//!
//! ```text
//! cargo run --release -p shoebox-ml --example ocr_check -- <models dir> <image or folder>...
//! ```
//!
//! The models directory holds `ocr-det-ppocrv5-mobile.onnx`, `ocr-rec-ppocrv5-latin.onnx` and
//! `ocr-dict-ppocrv5-latin.txt` (`cargo xtask models` writes them to `models/build`).

use anyhow::{bail, Result};
use shoebox_ml::PpOcr;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    if args.len() < 2 {
        bail!("usage: ocr_check <models dir> <image or folder>...");
    }
    let dir = Path::new(&args[0]);
    let t = Instant::now();
    let ocr = PpOcr::load(
        &dir.join("ocr-det-ppocrv5-mobile.onnx"),
        &dir.join("ocr-rec-ppocrv5-latin.onnx"),
        &dir.join("ocr-dict-ppocrv5-latin.txt"),
    )?;
    eprintln!("models read in {:.0?}", t.elapsed());
    for path in images(&args[1..])? {
        let bytes = std::fs::read(&path)?;
        let t = Instant::now();
        let text = shoebox_search::ocr::Ocr::read_text(&ocr, &bytes)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let lines: Vec<_> = text
            .lines()
            .map(|l| serde_json::json!({ "text": l }))
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "file": path.file_name().unwrap().to_string_lossy(),
                "total_ms": ms.round(),
                "lines": lines,
            })
        );
    }
    Ok(())
}
