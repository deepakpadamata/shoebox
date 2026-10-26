fn main() {
    // The commit this build comes from, for Settings → About (R1-05). Not a git checkout (a
    // source archive) or no git: "unknown".
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=10", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SHOEBOX_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git/refs/heads");
    println!("cargo:rerun-if-changed=../THIRD-PARTY-LICENSES.txt");

    #[cfg(feature = "tauri-app")]
    tauri_build::build();
}
