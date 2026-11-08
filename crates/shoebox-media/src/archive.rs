//! A set of archive parts (I1-04): `.zip`, `.tgz` / `.tar.gz`, or unpacked folders, listed without
//! extracting anything and read entry by entry as streams.
//!
//! A Takeout comes as several parts of up to 50 GB. Listing a zip reads its central directory only;
//! a `.tgz` has none, so listing streams through it once and reading streams through it again, in
//! order. Entries are never written out by path (a name like `../x` means nothing here): the
//! caller gets a reader and decides where the bytes go.

use crate::takeout::Entry;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

/// The most a JSON file (a sidecar, an album's metadata) may be: real ones are about 1 KB.
pub const MAX_JSON_BYTES: u64 = 4 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Zip,
    Tgz,
    Folder,
}

/// One entry of the set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Part, path, size and CRC, as the Takeout reader takes them.
    pub entry: Entry,
    /// The entry's modification time as the archive records it (UTC ms; zip times have no zone
    /// and are taken as UTC).
    pub modified_ms: Option<i64>,
    /// Its position in its part (the zip index; the order in a tar; the walk order in a folder).
    index: usize,
}

/// A part that could not be opened or listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartError {
    pub part: PathBuf,
    pub error: String,
}

pub struct ArchiveSet {
    parts: Vec<(PathBuf, Kind)>,
    entries: Vec<ArchiveEntry>,
}

fn kind_of(path: &Path) -> io::Result<Kind> {
    if path.is_dir() {
        return Ok(Kind::Folder);
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if name.ends_with(".zip") {
        Ok(Kind::Zip)
    } else if name.ends_with(".tgz") || name.ends_with(".tar.gz") {
        Ok(Kind::Tgz)
    } else if !path.exists() {
        Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
    } else {
        Err(io::Error::other("not a .zip, .tgz or folder"))
    }
}

fn zip_ms(t: Option<zip::DateTime>) -> Option<i64> {
    let t = t?;
    let days = days_from_civil(i64::from(t.year()), t.month().into(), t.day().into());
    let secs = days * 86_400
        + i64::from(t.hour()) * 3600
        + i64::from(t.minute()) * 60
        + i64::from(t.second());
    Some(secs * 1000)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn list_zip(path: &Path, part: usize, out: &mut Vec<ArchiveEntry>) -> io::Result<()> {
    let mut zip =
        zip::ZipArchive::new(BufReader::new(File::open(path)?)).map_err(io::Error::other)?;
    for index in 0..zip.len() {
        let f = zip.by_index_raw(index).map_err(io::Error::other)?;
        if f.is_dir() {
            continue;
        }
        out.push(ArchiveEntry {
            entry: Entry {
                part,
                path: f.name().map_err(io::Error::other)?.into_owned(),
                size: f.size(),
                crc32: Some(f.crc32()),
            },
            modified_ms: zip_ms(f.last_modified()),
            index,
        });
    }
    Ok(())
}

fn tar_reader(path: &Path) -> io::Result<tar::Archive<flate2::read::GzDecoder<BufReader<File>>>> {
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(
        BufReader::new(File::open(path)?),
    )))
}

fn list_tgz(path: &Path, part: usize, out: &mut Vec<ArchiveEntry>) -> io::Result<()> {
    let mut tar = tar_reader(path)?;
    for (index, e) in tar.entries()?.enumerate() {
        let e = e?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        out.push(ArchiveEntry {
            entry: Entry {
                part,
                path: e.path()?.to_string_lossy().replace('\\', "/"),
                size: e.size(),
                crc32: None,
            },
            modified_ms: e
                .header()
                .mtime()
                .ok()
                .and_then(|s| i64::try_from(s).ok())
                .map(|s| s * 1000),
            index,
        });
    }
    Ok(())
}

fn walk(root: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
        let mut items: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
        items.sort_by_key(|e| e.file_name());
        for e in items {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let t = e.file_type()?;
            let path = e.path();
            if t.is_dir() {
                visit(root, &path, out)?;
            } else if t.is_file() {
                let rel = path.strip_prefix(root).map_err(io::Error::other)?;
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    visit(root, root, &mut out)?;
    Ok(out)
}

fn list_folder(path: &Path, part: usize, out: &mut Vec<ArchiveEntry>) -> io::Result<()> {
    for (index, (rel, full)) in walk(path)?.into_iter().enumerate() {
        let meta = std::fs::metadata(&full)?;
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        out.push(ArchiveEntry {
            entry: Entry {
                part,
                path: rel,
                size: meta.len(),
                crc32: None,
            },
            modified_ms,
            index,
        });
    }
    Ok(())
}

impl ArchiveSet {
    /// Open and list the parts, in the order given. A part that is missing, unreadable or
    /// truncated (a zip without its central directory, a tgz cut short) is reported and left out;
    /// the others are listed.
    pub fn open(paths: &[PathBuf]) -> (Self, Vec<PartError>) {
        let mut set = ArchiveSet {
            parts: vec![],
            entries: vec![],
        };
        let mut errors = vec![];
        for path in paths {
            let part = set.parts.len();
            let mut listed = vec![];
            let result = kind_of(path).and_then(|kind| {
                match kind {
                    Kind::Zip => list_zip(path, part, &mut listed),
                    Kind::Tgz => list_tgz(path, part, &mut listed),
                    Kind::Folder => list_folder(path, part, &mut listed),
                }
                .map(|()| kind)
            });
            match result {
                Ok(kind) => {
                    set.parts.push((path.clone(), kind));
                    set.entries.extend(listed);
                }
                Err(e) => errors.push(PartError {
                    part: path.clone(),
                    error: e.to_string(),
                }),
            }
        }
        (set, errors)
    }

    /// The parts that were listed, in part order.
    pub fn parts(&self) -> impl Iterator<Item = &Path> {
        self.parts.iter().map(|(p, _)| p.as_path())
    }

    pub fn part_path(&self, part: usize) -> &Path {
        &self.parts[part].0
    }

    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// The entries as the Takeout reader takes them (same indices).
    pub fn takeout_entries(&self) -> Vec<Entry> {
        self.entries.iter().map(|e| e.entry.clone()).collect()
    }

    /// Call `f` for every entry of `wanted` (indices into [`entries`](Self::entries)) in `part`,
    /// in the part's own order, with a reader over the entry's bytes. A zip entry's CRC is checked
    /// when its reader is read to the end. Stops at the first error `f` returns; an entry that
    /// cannot be opened is handed to `f` as a reader that fails.
    pub fn visit(
        &self,
        part: usize,
        wanted: &BTreeSet<usize>,
        mut f: impl FnMut(usize, &mut dyn Read) -> io::Result<()>,
    ) -> io::Result<()> {
        let (path, kind) = &self.parts[part];
        let mut todo: Vec<(usize, usize)> = wanted
            .iter()
            .copied()
            .filter(|&i| self.entries.get(i).is_some_and(|e| e.entry.part == part))
            .map(|i| (self.entries[i].index, i))
            .collect();
        todo.sort_unstable();
        if todo.is_empty() {
            return Ok(());
        }
        match kind {
            Kind::Zip => {
                let mut zip = zip::ZipArchive::new(BufReader::new(File::open(path)?))
                    .map_err(io::Error::other)?;
                for (index, i) in todo {
                    match zip.by_index(index) {
                        Ok(mut r) => f(i, &mut r)?,
                        Err(e) => f(i, &mut Failing(Some(io::Error::other(e))))?,
                    }
                }
            }
            Kind::Tgz => {
                let mut tar = tar_reader(path)?;
                let mut next = todo.into_iter().peekable();
                for (index, e) in tar.entries()?.enumerate() {
                    let Some(&(want, i)) = next.peek() else { break };
                    if index < want {
                        continue;
                    }
                    next.next();
                    match e {
                        Ok(mut r) => f(i, &mut r)?,
                        Err(e) => f(i, &mut Failing(Some(e)))?,
                    }
                }
                for (_, i) in next {
                    f(
                        i,
                        &mut Failing(Some(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "the part ends early",
                        ))),
                    )?;
                }
            }
            Kind::Folder => {
                for (_, i) in todo {
                    let full = path.join(&self.entries[i].entry.path);
                    match File::open(&full) {
                        Ok(file) => f(i, &mut BufReader::new(file))?,
                        Err(e) => f(i, &mut Failing(Some(e)))?,
                    }
                }
            }
        }
        Ok(())
    }

    /// The bytes of small entries (JSON files), each at most [`MAX_JSON_BYTES`]; one pass per
    /// part. Entries that could not be read are missing from the result.
    pub fn read_small(
        &self,
        wanted: &BTreeSet<usize>,
    ) -> std::collections::HashMap<usize, Vec<u8>> {
        let mut out = std::collections::HashMap::new();
        for part in 0..self.parts.len() {
            let _ = self.visit(part, wanted, |i, r| {
                if self.entries[i].entry.size <= MAX_JSON_BYTES {
                    let mut bytes = vec![];
                    if r.take(MAX_JSON_BYTES + 1).read_to_end(&mut bytes).is_ok() {
                        out.insert(i, bytes);
                    }
                }
                Ok(())
            });
        }
        out
    }
}

/// A reader that fails with the error it was made with.
struct Failing(Option<io::Error>);

impl Read for Failing {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(self
            .0
            .take()
            .unwrap_or_else(|| io::Error::other("unreadable entry")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2024, 10, 27), 20_023);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }
}
