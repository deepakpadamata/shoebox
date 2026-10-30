use std::process::ExitCode;
use std::time::Instant;
use xtask::cli::{parse, Command, USAGE};
use xtask::corpus::{generate, Options};
use xtask::samples::write_samples;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Fixtures {
            preset,
            out,
            seed,
            metadata_only,
        } => {
            let start = Instant::now();
            match generate(&Options {
                preset,
                out_dir: out.clone(),
                seed,
                metadata_only,
            }) {
                Ok(s) => {
                    println!(
                        "{} photos ({} exact duplicates), {} files written to {} in {:.1?}\nmanifest: {}",
                        s.photos,
                        s.duplicates,
                        s.files_written,
                        out.display(),
                        start.elapsed(),
                        s.manifest.display()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Gazetteer { input, out } => match xtask::gazetteer::write(&input, &out) {
            Ok(s) => {
                println!(
                    "{} places, {} regions, {} countries: {} bytes written to {}",
                    s.places,
                    s.regions,
                    s.countries,
                    s.bytes,
                    out.display()
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Models { dir, check } => {
            let root = std::env::current_dir().expect("cwd");
            match xtask::models::run(&root, &dir, check) {
                Ok(r) => {
                    for (name, hash, size) in r.files {
                        println!("{name}: {size} bytes, BLAKE3 {hash}");
                    }
                    match r.written {
                        Some(p) => println!("wrote {}", p.display()),
                        None => println!("the built models match the compiled-in hashes"),
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Samples { out } => match write_samples(&out) {
            Ok(files) => {
                println!(
                    "wrote {} sample files to {}: {}",
                    files.len(),
                    out.display(),
                    files.join(", ")
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
