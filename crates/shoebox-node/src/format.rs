//! On-disk format versioning (M6-05).
//!
//! A data directory carries `format.json` with the number of the layout it was written in. On
//! opening, [`ensure_format`] makes the directory current: a fresh one is just stamped; an older
//! one (or a legacy one that predates the marker, version 0) is copied aside first and then
//! walked through the [`Migration`]s one step at a time, stamping after each step so a crash
//! resumes where it stopped; a newer one is refused untouched, so an old app never damages data
//! a newer app wrote.
//!
//! What the number covers: the files the app owns directly (`app.json`, `config.toml`, the
//! backup and album registries, the Loro `snapshot.loro`/`updates.log`, the docs store) and the
//! blob tag names. Wire protocols are versioned separately, by their ALPN (`shoebox/loro/0`): a
//! release that changes one serves the old ALPN as well as the new for one release, so versions N
//! and N-1 interoperate. Every name and file with its version is listed in `docs/notes/formats.md`
//! (a test keeps that list and the source in step).
//!
//! Layouts: 0 = before the marker; 1 = the marker (M6-05); 2 = **0.1.0, the first release**: the
//! latest epoch notice lives in `epoch-notice.bin` (either kind of epoch change), where layout 1
//! kept a revocation notice in `rotation-notice.bin`.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MARKER: &str = "format.json";
/// Files larger than this are not copied into the pre-migration backup (blob stores, docs
/// databases): migrations must never rewrite those in place without their own safeguard.
const BACKUP_MAX_FILE: u64 = 64 * 1024 * 1024;
/// Directories never copied into the backup.
const BACKUP_SKIP: &[&str] = &["blobs", "posters", "backups"];

/// The layout this build writes.
pub const CURRENT_FORMAT: u32 = 2;

#[derive(Serialize, Deserialize)]
struct Marker {
    format: u32,
    /// The app version that last stamped it (for support, not for logic).
    written_by: String,
}

/// One upgrade step from layout `from` to `from + 1`.
pub struct Migration {
    pub from: u32,
    pub name: &'static str,
    pub run: fn(&Path) -> Result<()>,
}

/// The migrations this build knows, oldest first. Layout 0 (no marker) to 1 only adds the marker.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        from: 0,
        name: "adopt the format marker",
        run: |_| Ok(()),
    },
    Migration {
        from: 1,
        name: "keep the latest epoch notice in epoch-notice.bin",
        run: epoch_notice_file,
    },
];

/// Layout 1 → 2: `rotation-notice.bin` (a revocation notice) becomes `epoch-notice.bin` (an
/// [`EpochNotice`](shoebox_core::identity::EpochNotice)). The node's data dir is the marked dir
/// itself (keeper) or its `node/` (app); both are looked at. An unreadable old file is dropped:
/// the notice only helps devices that were away, and they also get it from any other member.
fn epoch_notice_file(dir: &Path) -> Result<()> {
    use shoebox_core::identity::{EpochNotice, RotationNotice};
    for node_dir in [dir.to_path_buf(), dir.join("node")] {
        let old = node_dir.join("rotation-notice.bin");
        let Ok(bytes) = std::fs::read(&old) else {
            continue;
        };
        if let Ok(notice) = postcard::from_bytes::<RotationNotice>(&bytes) {
            let new = postcard::to_stdvec(&EpochNotice::Revocation(notice))?;
            let tmp = node_dir.join("epoch-notice.tmp");
            std::fs::write(&tmp, new)?;
            std::fs::rename(tmp, node_dir.join("epoch-notice.bin"))?;
        }
        std::fs::remove_file(old)?;
    }
    Ok(())
}

/// What [`ensure_format`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct FormatReport {
    pub from: u32,
    pub to: u32,
    /// Where the pre-migration copy of the small files went, if a migration ran.
    pub backup: Option<PathBuf>,
    pub steps: Vec<&'static str>,
}

fn read_marker(dir: &Path) -> Result<Option<u32>> {
    match std::fs::read(dir.join(MARKER)) {
        Ok(b) => {
            let m: Marker = serde_json::from_slice(&b).context("format.json is damaged")?;
            Ok(Some(m.format))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn stamp(dir: &Path, format: u32) -> Result<()> {
    let tmp = dir.join("format.json.tmp");
    let marker = Marker {
        format,
        written_by: env!("CARGO_PKG_VERSION").to_string(),
    };
    std::fs::write(&tmp, serde_json::to_vec_pretty(&marker)?)?;
    std::fs::rename(tmp, dir.join(MARKER))?;
    Ok(())
}

fn is_empty(dir: &Path) -> Result<bool> {
    Ok(std::fs::read_dir(dir)?.next().is_none())
}

fn backup(dir: &Path, from: u32) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = dir.join("backups").join(format!("format-{from}-{stamp}"));
    fn copy(src: &Path, dst: &Path) -> Result<()> {
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let (path, name) = (entry.path(), entry.file_name());
            let ty = entry.file_type()?;
            if ty.is_dir() {
                if BACKUP_SKIP.iter().any(|s| name == *s) {
                    continue;
                }
                std::fs::create_dir_all(dst.join(&name))?;
                copy(&path, &dst.join(&name))?;
            } else if ty.is_file() && entry.metadata()?.len() <= BACKUP_MAX_FILE {
                std::fs::copy(&path, dst.join(&name))?;
            }
        }
        Ok(())
    }
    std::fs::create_dir_all(&dest)?;
    copy(dir, &dest).context("copying the data aside before upgrading")?;
    Ok(dest)
}

/// Bring `dir` to layout `current` using `migrations`. See the module docs.
pub fn ensure_format_with(
    dir: &Path,
    current: u32,
    migrations: &[Migration],
) -> Result<FormatReport> {
    std::fs::create_dir_all(dir)?;
    let found = read_marker(dir)?;
    let from = match found {
        Some(v) => v,
        None if is_empty(dir)? => {
            stamp(dir, current)?;
            return Ok(FormatReport {
                from: current,
                to: current,
                backup: None,
                steps: vec![],
            });
        }
        None => 0, // data from before the marker existed
    };
    if from > current {
        bail!(
            "This data was written by a newer version of Shoebox (layout {from}); this version \
             understands up to layout {current}. Update the app: nothing was changed."
        );
    }
    if from == current {
        return Ok(FormatReport {
            from,
            to: current,
            backup: None,
            steps: vec![],
        });
    }
    let saved = backup(dir, from)?;
    let mut steps = vec![];
    for v in from..current {
        let m = migrations
            .iter()
            .find(|m| m.from == v)
            .ok_or_else(|| anyhow!("no migration from layout {v}: the data cannot be upgraded"))?;
        (m.run)(dir).with_context(|| {
            format!(
                "upgrading from layout {v} failed ({}); the copy from before the upgrade is in {}",
                m.name,
                saved.display()
            )
        })?;
        stamp(dir, v + 1)?;
        steps.push(m.name);
    }
    Ok(FormatReport {
        from,
        to: current,
        backup: Some(saved),
        steps,
    })
}

/// [`ensure_format_with`] for this build.
pub fn ensure_format(dir: &Path) -> Result<FormatReport> {
    ensure_format_with(dir, CURRENT_FORMAT, MIGRATIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(from: u32, name: &'static str, run: fn(&Path) -> Result<()>) -> Migration {
        Migration { from, name, run }
    }

    /// Pre-0.1 data (layout 1) keeps its revocation notice, now as an epoch notice.
    #[test]
    fn layout_1_notices_move_to_the_epoch_notice_file() {
        use shoebox_core::identity::*;
        use shoebox_core::model::Role;
        use shoebox_core::store::{Capability, LibraryCapability, LibraryId};
        let (root, admin) = (KeyPair::generate(), KeyPair::generate());
        let identity = LibraryId([1; 16]);
        let hlc = |ms| shoebox_core::Hlc {
            physical_ms: ms,
            counter: 0,
            device: admin.endpoint_id(),
        };
        let notice = issue_rotation_notice(
            &admin,
            identity,
            identity,
            LibraryCapability {
                library: LibraryId([2; 16]),
                root: root.root_public(),
                mode: Capability::Write,
                backend: vec![],
            },
            issue_device_cert(
                &root,
                identity,
                KeyPair::generate().endpoint_id(),
                Role::Companion,
                "p",
                hlc(5),
                true,
            ),
            issue_admin_cert(&root, identity, admin.endpoint_id(), hlc(1), None),
        );
        let d = tempfile::tempdir().unwrap();
        stamp(d.path(), 1).unwrap();
        std::fs::create_dir_all(d.path().join("node")).unwrap();
        std::fs::write(
            d.path().join("node/rotation-notice.bin"),
            postcard::to_stdvec(&notice).unwrap(),
        )
        .unwrap();
        // A keeper keeps the node files in the marked dir itself; this one is damaged.
        std::fs::write(d.path().join("rotation-notice.bin"), b"junk").unwrap();

        let r = ensure_format(d.path()).unwrap();
        assert_eq!((r.from, r.to), (1, 2));
        let moved: EpochNotice =
            postcard::from_bytes(&std::fs::read(d.path().join("node/epoch-notice.bin")).unwrap())
                .unwrap();
        assert_eq!(moved, EpochNotice::Revocation(notice));
        assert!(!d.path().join("node/rotation-notice.bin").exists());
        assert!(
            !d.path().join("rotation-notice.bin").exists(),
            "damaged one dropped"
        );
        assert!(!d.path().join("epoch-notice.bin").exists());
        assert_eq!(read_marker(d.path()).unwrap(), Some(2));
    }

    #[test]
    fn a_fresh_directory_is_stamped_and_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let r = ensure_format(d.path()).unwrap();
        assert_eq!(
            (r.from, r.to, r.backup),
            (CURRENT_FORMAT, CURRENT_FORMAT, None)
        );
        assert_eq!(read_marker(d.path()).unwrap(), Some(CURRENT_FORMAT));
        assert!(
            ensure_format(d.path()).unwrap().steps.is_empty(),
            "idempotent"
        );
    }

    #[test]
    fn data_from_before_the_marker_is_copied_aside_and_adopted() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("app.json"), b"{\"keep\":1}").unwrap();
        std::fs::create_dir_all(d.path().join("blobs")).unwrap();
        std::fs::write(d.path().join("blobs/big"), b"x").unwrap();
        let r = ensure_format(d.path()).unwrap();
        assert_eq!((r.from, r.to), (0, CURRENT_FORMAT));
        let b = r.backup.expect("a backup was made");
        assert_eq!(std::fs::read(b.join("app.json")).unwrap(), b"{\"keep\":1}");
        assert!(!b.join("blobs").exists(), "blob stores are not copied");
        assert_eq!(
            std::fs::read(d.path().join("app.json")).unwrap(),
            b"{\"keep\":1}"
        );
    }

    #[test]
    fn newer_data_is_refused_untouched() {
        let d = tempfile::tempdir().unwrap();
        stamp(d.path(), CURRENT_FORMAT + 1).unwrap();
        std::fs::write(d.path().join("app.json"), b"new").unwrap();
        let err = ensure_format(d.path()).unwrap_err().to_string();
        assert!(
            err.contains("newer version") && err.contains("nothing was changed"),
            "{err}"
        );
        assert_eq!(read_marker(d.path()).unwrap(), Some(CURRENT_FORMAT + 1));
        assert!(!d.path().join("backups").exists());
    }

    #[test]
    fn steps_run_in_order_and_a_failure_keeps_the_last_good_stamp() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("app.json"), b"v1").unwrap();
        stamp(d.path(), 1).unwrap();
        let ok = [
            step(1, "one", |p| Ok(std::fs::write(p.join("a"), "1")?)),
            step(2, "two", |p| {
                assert!(p.join("a").exists(), "step one ran first");
                Ok(std::fs::write(p.join("b"), "2")?)
            }),
        ];
        let r = ensure_format_with(d.path(), 3, &ok).unwrap();
        assert_eq!((r.steps.clone(), r.to), (vec!["one", "two"], 3));

        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("app.json"), b"v1").unwrap();
        stamp(d.path(), 1).unwrap();
        let bad = [
            step(1, "fine", |_| Ok(())),
            step(2, "breaks", |_| bail!("disk on fire")),
        ];
        let err = format!("{:#}", ensure_format_with(d.path(), 3, &bad).unwrap_err());
        assert!(
            err.contains("disk on fire") && err.contains("backups"),
            "{err}"
        );
        assert_eq!(
            read_marker(d.path()).unwrap(),
            Some(2),
            "resumes from the failed step"
        );
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("app.json"), b"v3").unwrap();
        stamp(d.path(), 3).unwrap();
        let missing = ensure_format_with(d.path(), 5, &bad)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("no migration from layout 3"), "{missing}");
    }
}
