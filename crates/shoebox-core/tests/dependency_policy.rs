//! `shoebox-core` must stay free of async runtimes and iroh (M0-04), so it remains pure and the
//! store backend swappable.

#[test]
fn core_has_no_async_runtime_or_iroh_dependency() {
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
        "futures",
        "async-trait",
    ] {
        assert!(
            !deps.iter().any(|l| l.trim_start().starts_with(forbidden)),
            "shoebox-core must not depend on `{forbidden}`"
        );
    }
}
