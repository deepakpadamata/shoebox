//! Write `labels.bin` (S1-13): every label of `labels.txt` embedded by the shipped text model in
//! the prompt "a photo of a {label}". Run by `cargo xtask models`.
//! `cargo run --release -p shoebox-ml --example label_embeddings -- <models dir> <labels.txt> <labels.bin>`

use shoebox_ml::ClipText;
use shoebox_search::encoder::TextEncoder;
use shoebox_search::labels::{encode_vectors, parse_list, LABELS_VERSION, PROMPT};
use shoebox_search::shard::Vec8;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, list, out] = &args[..] else {
        anyhow::bail!("usage: label_embeddings <models dir> <labels.txt> <labels.bin>");
    };
    let text = ClipText::bundled(&[PathBuf::from(dir)])?;
    let labels = parse_list(&std::fs::read_to_string(list)?);
    let mut vectors = vec![];
    for (name, _) in &labels {
        let v = text
            .encode_text(&PROMPT.replace("{}", name))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        vectors.push(Vec8::quantize(&v));
    }
    let bytes = encode_vectors(text.model(), LABELS_VERSION, &vectors, text.dims() as u16);
    std::fs::write(out, &bytes)?;
    println!("{} labels, {} bytes", labels.len(), bytes.len());
    Ok(())
}
