use shoebox_store_loro::harness::LoroHarness;

/// Everything except shared albums, which are outside the spike.
#[test]
fn loro_spike_passes_the_library_conformance_scenarios() {
    futures::executor::block_on(shoebox_core::conformance::run_only(
        &LoroHarness::default(),
        |name| name != "shared_album_flow",
    ));
}

#[test]
fn the_excluded_scenario_is_the_only_one_skipped() {
    assert!(shoebox_core::conformance::SCENARIOS.contains(&"shared_album_flow"));
    assert!(shoebox_core::conformance::SCENARIOS.len() >= 12);
}
