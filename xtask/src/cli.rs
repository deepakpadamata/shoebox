//! Argument parsing for `cargo xtask`.

use crate::corpus::Preset;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Command {
    Fixtures {
        preset: Preset,
        out: PathBuf,
        seed: u64,
        metadata_only: Option<bool>,
    },
    Samples {
        out: PathBuf,
    },
    Help,
}

pub const USAGE: &str = "\
usage:
  cargo xtask fixtures [--preset small|medium|large] [--out DIR] [--seed N]
                       [--metadata-only | --with-files]
      Generate a synthetic photo corpus (default: medium into fixtures-out/<preset>).
      `large` is metadata-only by default (manifest only, for benchmarks).
  cargo xtask samples [--out DIR]
      Regenerate the checked-in sample files (default: fixtures/samples).";

pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter().map(String::as_str);
    match it.next() {
        None | Some("help" | "--help" | "-h") => Ok(Command::Help),
        Some("fixtures") => {
            let (mut preset, mut out, mut seed, mut meta) = (Preset::Medium, None, 1u64, None);
            while let Some(flag) = it.next() {
                let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
                match flag {
                    "--preset" => {
                        let v = value("--preset")?;
                        preset = Preset::parse(v).ok_or(format!("unknown preset `{v}`"))?;
                    }
                    "--out" => out = Some(PathBuf::from(value("--out")?)),
                    "--seed" => {
                        seed = value("--seed")?
                            .parse()
                            .map_err(|_| "--seed must be a number")?
                    }
                    "--metadata-only" => meta = Some(true),
                    "--with-files" => meta = Some(false),
                    other => return Err(format!("unknown flag `{other}`")),
                }
            }
            let out = out.unwrap_or_else(|| PathBuf::from("fixtures-out").join(preset.name()));
            Ok(Command::Fixtures {
                preset,
                out,
                seed,
                metadata_only: meta,
            })
        }
        Some("samples") => {
            let mut out = PathBuf::from("fixtures/samples");
            while let Some(flag) = it.next() {
                match flag {
                    "--out" => out = PathBuf::from(it.next().ok_or("--out needs a value")?),
                    other => return Err(format!("unknown flag `{other}`")),
                }
            }
            Ok(Command::Samples { out })
        }
        Some(other) => Err(format!("unknown task `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn fixtures_defaults_to_medium() {
        assert_eq!(
            parse(&args("fixtures")),
            Ok(Command::Fixtures {
                preset: Preset::Medium,
                out: PathBuf::from("fixtures-out/medium"),
                seed: 1,
                metadata_only: None
            })
        );
    }

    #[test]
    fn fixtures_flags() {
        assert_eq!(
            parse(&args(
                "fixtures --preset large --seed 9 --with-files --out /tmp/x"
            )),
            Ok(Command::Fixtures {
                preset: Preset::Large,
                out: PathBuf::from("/tmp/x"),
                seed: 9,
                metadata_only: Some(false)
            })
        );
        assert!(matches!(
            parse(&args("fixtures --metadata-only")),
            Ok(Command::Fixtures {
                metadata_only: Some(true),
                ..
            })
        ));
    }

    #[test]
    fn samples_and_errors() {
        assert_eq!(
            parse(&args("samples")),
            Ok(Command::Samples {
                out: PathBuf::from("fixtures/samples")
            })
        );
        assert_eq!(parse(&[]), Ok(Command::Help));
        assert!(parse(&args("fixtures --preset huge")).is_err());
        assert!(parse(&args("fixtures --seed")).is_err());
        assert!(parse(&args("fixtures --bogus")).is_err());
        assert!(parse(&args("nope")).is_err());
    }
}
