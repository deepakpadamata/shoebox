//! `shoebox-search` runs on every device, a Pi included, and must stay free of async runtimes,
//! networking and ML runtimes (S1-02): models are reached through traits the node implements.

#[test]
fn search_has_no_async_network_or_ml_dependency() {
    let manifest = include_str!("../Cargo.toml");
    let deps: Vec<&str> = manifest
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    for forbidden in [
        "tokio",
        "async-std",
        "smol",
        "iroh",
        "reqwest",
        "hyper",
        "ort",
        "tract",
        "candle",
        "onnx",
        "tflite",
        "tch",
    ] {
        assert!(
            !deps.iter().any(|l| l.trim_start().starts_with(forbidden)),
            "shoebox-search must not depend on `{forbidden}`"
        );
    }
}
