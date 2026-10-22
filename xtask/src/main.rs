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
