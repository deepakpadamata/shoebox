use shoebox_store_docs::harness::DocsHarness;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn docs_backend_passes_the_conformance_suite() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let harness = DocsHarness::default();
    shoebox_core::conformance::run(&harness).await;
    harness.shutdown().await;
}
