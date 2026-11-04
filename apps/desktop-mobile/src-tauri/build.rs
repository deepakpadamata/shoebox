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
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            shoebox_photos();
        }
        tauri_build::build();
    }
}

/// Compile the ShoeboxPhotos Swift package (P1-03) into static libraries and link them, for the
/// Photos library as a backup source on a Mac (P1-04, `macos_photos.rs`). `swiftc` directly, not
/// `swift build`: it needs no package manifest, so it works with Command Line Tools whose
/// manifest API is broken, as well as with Xcode.
#[cfg(feature = "tauri-app")]
fn shoebox_photos() {
    use std::path::PathBuf;
    use std::process::Command;
    let pkg =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../native/ShoeboxPhotos");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("shoebox-photos");
    std::fs::create_dir_all(&out).unwrap();
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        a => a,
    }
    .to_string();
    let target = format!("{arch}-apple-macos13");
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let sources = |dir: &str| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(pkg.join("Sources").join(dir))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "swift"))
            .collect();
        v.sort();
        v
    };
    for module in ["ShoeboxPhotos", "ShoeboxPhotosBridge"] {
        let mut cmd = Command::new("xcrun");
        cmd.args([
            "--sdk",
            "macosx",
            "swiftc",
            "-swift-version",
            "5",
            "-parse-as-library",
        ])
        .args(["-target", &target, "-module-name", module])
        .args(["-emit-library", "-static", "-emit-module"])
        .arg("-emit-module-path")
        .arg(out.join(format!("{module}.swiftmodule")))
        .arg("-I")
        .arg(&out)
        .arg("-o")
        .arg(out.join(format!("lib{module}.a")))
        .arg(if release { "-O" } else { "-Onone" })
        .args(sources(module));
        let status = cmd
            .status()
            .expect("swiftc (install Xcode or the Command Line Tools)");
        assert!(status.success(), "compiling {module} failed");
    }
    println!("cargo:rerun-if-changed={}", pkg.join("Sources").display());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=ShoeboxPhotosBridge");
    println!("cargo:rustc-link-lib=static=ShoeboxPhotos");
    for f in [
        "Photos",
        "AVFoundation",
        "CoreMedia",
        "ImageIO",
        "UniformTypeIdentifiers",
    ] {
        println!("cargo:rustc-link-lib=framework={f}");
    }
    // The Swift runtime's library paths (no packages: those are built above).
    swift_rs::SwiftLinker::new("13").link();
}
