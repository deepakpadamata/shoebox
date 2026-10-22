use shoebox_store_mem::harness::MemHarness;

#[test]
fn mem_backend_passes_the_conformance_suite() {
    futures::executor::block_on(shoebox_core::conformance::run(&MemHarness::default()));
}
