//! The documented M1 demo script runs end to end against the CLI.

use std::path::Path;
use std::process::Command;

#[test]
fn the_m1_demo_script_runs_end_to_end() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let state = tempfile::tempdir().unwrap();
    let out = Command::new("bash")
        .arg(root.join("docs/notes/demo-m1.sh"))
        .env("BIN", env!("CARGO_BIN_EXE_shoebox-dev"))
        .env("DEMO_DIR", state.path())
        .output()
        .expect("runs bash");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "demo failed:\n{stdout}\n{stderr}");
    assert!(stdout.contains("demo complete"), "{stdout}");
    // The story's key facts, in the CLI's own words.
    assert!(stdout.contains("imported 3"), "{stdout}");
    assert!(
        stdout.contains("joined; the library has 3 photo(s)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("2 copies (target 2)"),
        "the Pi's fetch made a second copy:\n{stdout}"
    );
    assert!(stdout.contains("reachable"), "{stdout}");
}
