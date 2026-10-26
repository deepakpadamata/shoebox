# Changelog

## 0.1.0 (unreleased): testers' preview

The first build for other people's devices (macOS, Android, the keeper on Linux). Written up for testers in R1-12.

- From this version on, wire protocols, signing domains and on-disk files are kept compatible: see
  [docs/notes/formats.md](docs/notes/formats.md) and "Changing a format" in [CONTRIBUTING.md](CONTRIBUTING.md).
- On-disk layout 2: data from earlier development builds is upgraded on first start (a copy of the small files is kept
  under `backups/`).
