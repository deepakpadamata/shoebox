//! Where a replica's memory goes at library scale (M4-11 / M6-02, `docs/notes/phone-memory.md`).
//!
//!     cargo run --release -p shoebox-store-loro --example phone_memory -- 100000
//!
//! A counting allocator gives live heap bytes (and the peak) for each part, built from the same
//! synthetic library: the records themselves, `LibraryState`, Loro's document (built by writes,
//! loaded from a snapshot, received by a sync) and the whole Loro backend receiving a sync.
//! Holdings: a keeper and a Mac each hold all three blobs of every photo (6 records a photo).

use cap::Cap;
use loro::{ExportMode, LoroDoc, LoroValue};
use shoebox_core::clock::ManualClock;
use shoebox_core::entry_codec::{encode_library_entry, library_key};
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::library_state::{LibraryState, StateEntry};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_store_loro::{LoroBackend, LoroNetwork};
use std::alloc::System;

#[global_allocator]
static A: Cap<System> = Cap::new(System, usize::MAX);

fn live() -> usize {
    A.allocated()
}

/// Starts a measurement: the live bytes now. The peak is the process's (run one part a process
/// for a meaningful peak).
fn mark() -> usize {
    live()
}

fn report(what: &str, base: usize, n: usize) -> usize {
    let (now, peak) = (live(), A.max_allocated());
    let d = now.saturating_sub(base);
    eprintln!(
        "{what:<58} {:>8.1} MB  {:>6} B/photo   peak +{:>7.1} MB",
        d as f64 / 1e6,
        d / n.max(1),
        peak.saturating_sub(base) as f64 / 1e6
    );
    d
}

fn dev(n: u8) -> EndpointId {
    EndpointId([n; 32])
}

fn id_of(i: usize, salt: &[u8]) -> Blake3Hash {
    let mut bytes = (i as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(salt);
    Blake3Hash::of_bytes(&bytes)
}

/// The store benchmark's photo: three JPEG blobs and a phone camera's EXIF summary.
fn photo(i: usize, n: usize) -> PhotoRecord {
    let blob = |h: Blake3Hash, mime: &str| BlobRef {
        hash: h,
        size: 4_000_000,
        mime: mime.into(),
    };
    let original = id_of(i, b"");
    PhotoRecord {
        id: original,
        original: blob(original, "image/jpeg"),
        preview: blob(id_of(i, b"p"), "image/jpeg"),
        thumb: blob(id_of(i, b"t"), "image/jpeg"),
        taken_at: Timestamp(1_400_000_000_000 + ((i * 7919) % n) as i64 * 37_000),
        exif: ExifSummary {
            camera_make: Some("Apple".into()),
            camera_model: Some("iPhone 15 Pro".into()),
            lens: Some("iPhone 15 Pro back triple camera 6.8mm f/1.78".into()),
            width: 4032,
            height: 3024,
            gps: Some(Gps {
                lat_e7: 515_074_000,
                lon_e7: -1_278_000,
            }),
        },
        imported_by: dev(1),
        favorite: Lww::new(false, Hlc::GENESIS),
        deleted_at: Lww::new(None, Hlc::GENESIS),
    }
}

const HOLDERS: [u8; 2] = [7, 8];

fn holdings(i: usize) -> Vec<StateEntry> {
    let mut out = vec![];
    for d in HOLDERS {
        for (salt, kind) in [
            (&b""[..], BlobKind::Original),
            (b"p", BlobKind::Preview),
            (b"t", BlobKind::Thumb),
        ] {
            out.push(StateEntry::Holding {
                device: dev(d),
                blob: id_of(i, salt),
                kind: Some(kind),
                hlc: Hlc {
                    physical_ms: 1_700_000_000_000 + i as u64,
                    counter: 0,
                    device: dev(d),
                },
            });
        }
    }
    out
}

/// `<author>/<record key>` → `0x01 ++ signature(64) ++ value`, as the backend stores a record.
fn stored(e: &StateEntry, author: &EndpointId) -> (String, Vec<u8>) {
    let key = format!("{}/{}", hex::encode(&author.0[..8]), library_key(e));
    let mut v = vec![1u8];
    v.extend_from_slice(&[0xA5; 64]);
    v.extend_from_slice(&encode_library_entry(e));
    (key, v)
}

fn author_of(e: &StateEntry) -> EndpointId {
    match e {
        StateEntry::Holding { device, .. } => *device,
        _ => dev(1),
    }
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let only = std::env::args().nth(2).unwrap_or_default();
    eprintln!("{n} photos, {} holdings each\n", HOLDERS.len() * 3);

    if only.is_empty() || only == "fields" {
        fields(n);
    }
    if only.is_empty() || only == "state" {
        state(n);
    }
    if only.is_empty() || only == "loro" {
        loro_doc(n, Layout::Today);
    }
    if only == "loro-photos" {
        loro_doc(n, Layout::PhotosOnly);
    }
    if only == "loro-per-device" {
        loro_doc(n, Layout::PerDevicePhoto);
    }
    if only == "loro-bucketed" {
        loro_doc(n, Layout::Bucketed);
    }
    if only.is_empty() || only == "windowed" {
        windowed_state(n);
    }
    // Each in its own process, so that the peak is theirs.
    match only.as_str() {
        "build" => build(n),
        "sync" => sync(n),
        "open" => open(n),
        _ => {}
    }
}

/// Each field of `PhotoRecord`, and the record as the app holds it.
fn fields(n: usize) {
    use std::mem::size_of;
    eprintln!("-- PhotoRecord --");
    eprintln!(
        "size_of PhotoRecord            {:>4} B",
        size_of::<PhotoRecord>()
    );
    eprintln!(
        "  id (PhotoId)                 {:>4} B",
        size_of::<PhotoId>()
    );
    eprintln!(
        "  original/preview/thumb       {:>4} B each (hash 32, size 8, mime String 24) + mime heap",
        size_of::<BlobRef>()
    );
    eprintln!(
        "  taken_at                     {:>4} B",
        size_of::<Timestamp>()
    );
    eprintln!(
        "  exif                         {:>4} B + make/model/lens heap",
        size_of::<ExifSummary>()
    );
    eprintln!(
        "  imported_by                  {:>4} B",
        size_of::<EndpointId>()
    );
    eprintln!(
        "  favorite Lww<bool>           {:>4} B (Hlc {} B)",
        size_of::<Lww<bool>>(),
        size_of::<Hlc>()
    );
    eprintln!(
        "  deleted_at Lww<Option<Ts>>   {:>4} B",
        size_of::<Lww<Option<Timestamp>>>()
    );
    let p = photo(0, n);
    let heap = 3 * p.original.mime.capacity()
        + p.exif.camera_make.as_ref().map_or(0, |s| s.capacity())
        + p.exif.camera_model.as_ref().map_or(0, |s| s.capacity())
        + p.exif.lens.as_ref().map_or(0, |s| s.capacity());
    eprintln!("  heap strings (this photo)    {heap:>4} B");
    eprintln!(
        "  encoded record (postcard)    {:>4} B; stored in Loro with key + signature: key {} B, value {} B",
        encode_library_entry(&StateEntry::Photo(p.clone())).len(),
        stored(&StateEntry::Photo(p.clone()), &dev(1)).0.len(),
        stored(&StateEntry::Photo(p), &dev(1)).1.len()
    );
    let h = &holdings(0)[0];
    let (k, v) = stored(h, &dev(7));
    eprintln!(
        "  one holding stored in Loro   key {} B, value {} B",
        k.len(),
        v.len()
    );

    let base = mark();
    let v: Vec<PhotoRecord> = (0..n).map(|i| photo(i, n)).collect();
    report(
        "Vec<PhotoRecord> (all records, as photos_by_time(all))",
        base,
        n,
    );
    drop(v);
    eprintln!();
}

/// `LibraryState` holding the library: photos, then holdings.
fn state(n: usize) {
    eprintln!("-- LibraryState --");
    let base = mark();
    let mut s = LibraryState::new(dev(9));
    for i in 0..n {
        s.apply_entry(&StateEntry::Photo(photo(i, n)));
    }
    let photos = report("photos (map of records + timeline)", base, n);
    let base2 = mark();
    for i in 0..n {
        for h in holdings(i) {
            s.apply_entry(&h);
        }
    }
    let hold = report(
        "+ 6 holdings a photo (holdings + holders_by_blob)",
        base2,
        n,
    );
    eprintln!(
        "{:<58} {:>8.1} MB  {:>6} B/photo\n",
        "LibraryState total",
        (photos + hold) as f64 / 1e6,
        (photos + hold) / n
    );
    drop(s);
}

/// How records are laid out in the Loro map.
#[derive(Clone, Copy, PartialEq)]
enum Layout {
    /// As today: a photo record, and one record per (device, blob) held.
    Today,
    /// Photo records only: the floor for any layout with a record per photo.
    PhotosOnly,
    /// A photo record, and one holdings record per (device, photo): the kinds held as bits.
    PerDevicePhoto,
    /// A photo record, and holdings in buckets of 256 photos per device.
    Bucketed,
}

fn records(n: usize, layout: Layout) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![];
    for i in 0..n {
        out.push(stored(&StateEntry::Photo(photo(i, n)), &dev(1)));
        match layout {
            Layout::Today => out.extend(holdings(i).iter().map(|e| stored(e, &author_of(e)))),
            Layout::PhotosOnly | Layout::Bucketed => {}
            Layout::PerDevicePhoto => {
                for d in HOLDERS {
                    let key = format!(
                        "{}/held/{}",
                        hex::encode(&dev(d).0[..8]),
                        hex::encode(id_of(i, b"").0)
                    );
                    // signature, kinds bits, hlc (ms + counter; the device is the author)
                    let mut v = vec![1u8];
                    v.extend_from_slice(&[0xA5; 64]);
                    v.push(0b111);
                    v.extend_from_slice(&(1_700_000_000_000u64 + i as u64).to_le_bytes());
                    v.extend_from_slice(&0u32.to_le_bytes());
                    out.push((key, v));
                }
            }
        }
    }
    if layout == Layout::Bucketed {
        for d in HOLDERS {
            for b in 0..n.div_ceil(256) {
                let key = format!("{}/held/{b:05x}", hex::encode(&dev(d).0[..8]));
                let mut v = vec![1u8];
                v.extend_from_slice(&[0xA5; 64]);
                for i in b * 256..((b + 1) * 256).min(n) {
                    v.extend_from_slice(&id_of(i, b"").0);
                    v.push(0b111);
                }
                out.push((key, v));
            }
        }
    }
    out
}

/// Loro's document on its own, holding the same records under the same keys.
fn loro_doc(n: usize, layout: Layout) {
    let name = match layout {
        Layout::Today => "today's layout: photo + 6 holding records",
        Layout::PhotosOnly => "photo records only",
        Layout::PerDevicePhoto => "photo + 1 holdings record per device",
        Layout::Bucketed => "photo + holdings in buckets of 256",
    };
    eprintln!("-- Loro document, {name} --");
    // Marked before the records are made: they are moved into the document (or freed) below.
    let base = mark();
    let recs = records(n, layout);
    let raw: usize = recs.iter().map(|(k, v)| k.len() + v.len()).sum();
    eprintln!(
        "  {} records, keys + values {:.1} MB ({} B/photo)",
        recs.len(),
        raw as f64 / 1e6,
        raw / n
    );
    let doc = LoroDoc::new();
    let map = doc.get_map("entries");
    for (j, (k, v)) in recs.into_iter().enumerate() {
        map.insert(&k, LoroValue::Binary(v.into())).unwrap();
        // The backend commits after each write; batching here keeps the build fast.
        if j % 64 == 63 {
            doc.commit();
        }
    }
    doc.commit();
    report("built by local writes (history + state)", base, n);
    let snapshot = doc.export(ExportMode::Snapshot).unwrap();
    let updates = doc.export(ExportMode::all_updates()).unwrap();
    let shallow = doc
        .export(ExportMode::shallow_snapshot(&doc.oplog_frontiers()))
        .unwrap();
    eprintln!(
        "  snapshot {:.1} MB, all updates {:.1} MB, shallow snapshot {:.1} MB",
        snapshot.len() as f64 / 1e6,
        updates.len() as f64 / 1e6,
        shallow.len() as f64 / 1e6
    );
    drop(map);
    drop(doc);

    let t = std::time::Instant::now();
    let base = mark();
    let d = LoroDoc::new();
    d.import(&snapshot).unwrap();
    report("loaded from a snapshot (app start), untouched", base, n);
    let m = d.get_map("entries");
    let mut count = 0;
    m.for_each(|_, _| count += 1);
    report("  ... after reading every value (fold_all)", base, n);
    eprintln!("  ({:.2?})", t.elapsed());
    d.compact_change_store();
    d.free_history_cache();
    d.free_diff_calculator();
    report("  ... after compact_change_store + free caches", base, n);
    drop(m);
    drop(d);

    let t = std::time::Instant::now();
    let base = mark();
    let d = LoroDoc::new();
    d.import(&updates).unwrap();
    report("received by a sync into an empty doc", base, n);
    let m = d.get_map("entries");
    m.for_each(|_, _| ());
    report("  ... after reading every value", base, n);
    eprintln!("  ({:.2?})", t.elapsed());
    drop(m);
    drop(d);

    let base = mark();
    let d = LoroDoc::new();
    d.import(&shallow).unwrap();
    let m = d.get_map("entries");
    m.for_each(|_, _| ());
    report("shallow snapshot (state, no history), values read", base, n);
    drop(m);
    drop(d);
    drop((snapshot, updates, shallow));
    eprintln!();
}

/// The windowed state (prototype): what `LibraryState` would keep per photo, with full records
/// decoded from the Loro document on demand.
mod windowed {
    use super::*;
    use std::cmp::Reverse;
    use std::collections::{BTreeSet, HashMap};

    pub const FAVORITE: u8 = 1;
    pub const DELETED: u8 = 2;

    pub struct Columns {
        pub index: HashMap<PhotoId, u32>,
        pub ids: Vec<PhotoId>,
        pub taken_at: Vec<i64>,
        pub flags: Vec<u8>,
        pub thumb: Vec<Blake3Hash>,
        /// Live photos newest first, as indexes into the columns.
        pub timeline: BTreeSet<(Reverse<i64>, u32)>,
        /// Per device (by position in the device list): the kinds held of each photo, as bits.
        pub held: Vec<Vec<u8>>,
    }

    impl Columns {
        pub fn new(devices: usize) -> Self {
            Self {
                index: HashMap::new(),
                ids: vec![],
                taken_at: vec![],
                flags: vec![],
                thumb: vec![],
                timeline: BTreeSet::new(),
                held: vec![vec![]; devices],
            }
        }

        pub fn upsert(&mut self, p: &PhotoRecord) {
            let flags = (u8::from(p.favorite.value) * FAVORITE)
                | (u8::from(p.deleted_at.value.is_some()) * DELETED);
            let i = match self.index.get(&p.id) {
                Some(&i) => {
                    self.timeline
                        .remove(&(Reverse(self.taken_at[i as usize]), i));
                    i
                }
                None => {
                    let i = self.ids.len() as u32;
                    self.index.insert(p.id, i);
                    self.ids.push(p.id);
                    self.taken_at.push(0);
                    self.flags.push(0);
                    self.thumb.push(p.thumb.hash);
                    for h in &mut self.held {
                        h.push(0);
                    }
                    i
                }
            };
            self.taken_at[i as usize] = p.taken_at.0;
            self.flags[i as usize] = flags;
            if flags & DELETED == 0 {
                self.timeline.insert((Reverse(p.taken_at.0), i));
            }
        }

        pub fn shrink(&mut self) {
            self.ids.shrink_to_fit();
            self.taken_at.shrink_to_fit();
            self.flags.shrink_to_fit();
            self.thumb.shrink_to_fit();
            for h in &mut self.held {
                h.shrink_to_fit();
            }
        }

        /// The first `limit` live photos, newest first: ids from the columns, records decoded.
        pub fn page(
            &self,
            limit: usize,
            load: impl Fn(&PhotoId) -> Option<PhotoRecord>,
        ) -> Vec<PhotoRecord> {
            self.timeline
                .iter()
                .take(limit)
                .filter_map(|(_, i)| load(&self.ids[*i as usize]))
                .collect()
        }
    }
}

/// The windowed state on top of Loro's document as it is today (records read from a snapshot).
fn windowed_state(n: usize) {
    use shoebox_core::entry_codec::decode_library_entry;
    eprintln!("-- Windowed state (prototype) --");
    let base = mark();
    let mut c = windowed::Columns::new(HOLDERS.len());
    for i in 0..n {
        c.upsert(&photo(i, n));
        for (d, _) in HOLDERS.iter().enumerate() {
            c.held[d][i] = 0b111;
        }
    }
    c.shrink();
    report(
        "columns: id, taken_at, flags, thumb + timeline + held bits",
        base,
        n,
    );
    let base2 = mark();
    let mut blobs: std::collections::HashMap<Blake3Hash, u32> = Default::default();
    for i in 0..n {
        blobs.insert(id_of(i, b"p"), i as u32);
        blobs.insert(id_of(i, b"t"), i as u32);
    }
    report(
        "  + preview/thumb hash -> photo (for holders(blob))",
        base2,
        n,
    );
    drop(blobs);

    // Records on demand, from Loro's document.
    let doc = LoroDoc::new();
    let map = doc.get_map("entries");
    for (k, v) in records(n, Layout::PhotosOnly) {
        map.insert(&k, LoroValue::Binary(v.into())).unwrap();
    }
    doc.commit();
    let prefix = hex::encode(&dev(1).0[..8]);
    let load = |id: &PhotoId| -> Option<PhotoRecord> {
        let key = format!("{prefix}/photos/{}", hex::encode(id.0));
        match map.get(&key) {
            Some(loro::ValueOrContainer::Value(LoroValue::Binary(b))) => {
                match decode_library_entry(&key[17..], &b[65..]).ok()? {
                    StateEntry::Photo(p) => Some(p),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    let t = std::time::Instant::now();
    let page = c.page(200, load);
    eprintln!(
        "  first timeline page of {} decoded from Loro in {:.2?}",
        page.len(),
        t.elapsed()
    );
    let t = std::time::Instant::now();
    let all: usize = c.timeline.len();
    let mut decoded = 0;
    for (_, i) in c.timeline.iter() {
        decoded += usize::from(load(&c.ids[*i as usize]).is_some());
    }
    eprintln!(
        "  every record decoded once ({decoded}/{all}): {:.2?}\n",
        t.elapsed()
    );
}

fn cap() -> LibraryCapability {
    LibraryCapability {
        library: LibraryId([4; 16]),
        root: RootPublicKey([3; 32]),
        mode: Capability::Write,
        backend: vec![shoebox_store_loro::CAPABILITY_VERSION],
    }
}

fn work_dir() -> std::path::PathBuf {
    std::env::var_os("PHONE_MEMORY_DIR")
        .map(Into::into)
        .unwrap_or_else(|| std::env::temp_dir().join("shoebox-phone-memory"))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

/// Builds the library the way devices write it (photos by one device, holdings by two others,
/// each through its own replica) and saves a snapshot of it for `sync` and `open`. Records are
/// unsigned here: the in-process backend has no keys (64 B a record less than on a device).
fn build(n: usize) {
    runtime().block_on(async {
        let net = LoroNetwork::new();
        let clock = ManualClock::new(1_700_000_000_000);
        let a = LoroBackend::new(net.clone(), dev(1), clock.clone());
        let lib_a = a.open_library(&cap()).await.unwrap();
        for i in 0..n {
            lib_a.upsert_photo(photo(i, n)).await.unwrap();
        }
        for d in HOLDERS {
            let b = LoroBackend::new(net.clone(), dev(d), clock.clone());
            let lib = b.open_library(&cap()).await.unwrap();
            lib.sync_with(&PeerAddr::id_only(dev(1))).await.unwrap();
            for chunk in (0..n).collect::<Vec<_>>().chunks(5_000) {
                let mut added = vec![];
                for &i in chunk {
                    added.push((id_of(i, b""), BlobKind::Original));
                    added.push((id_of(i, b"p"), BlobKind::Preview));
                    added.push((id_of(i, b"t"), BlobKind::Thumb));
                }
                lib.put_holdings(
                    dev(d),
                    HoldingsDelta {
                        hlc: Hlc {
                            physical_ms: 1_700_000_000_000 + chunk[0] as u64,
                            counter: 0,
                            device: dev(d),
                        },
                        added,
                        removed: vec![],
                    },
                )
                .await
                .unwrap();
            }
            lib_a.sync_with(&PeerAddr::id_only(dev(d))).await.unwrap();
        }
        let snap = lib_a.export_state().await.unwrap();
        std::fs::create_dir_all(work_dir()).unwrap();
        std::fs::write(work_dir().join("library.snapshot"), &snap).unwrap();
        eprintln!(
            "built {n} photos; snapshot {:.1} MB in {}",
            snap.len() as f64 / 1e6,
            work_dir().display()
        );
    });
}

/// A phone with an empty, persisted replica receives the whole library.
fn sync(n: usize) {
    let snap = std::fs::read(work_dir().join("library.snapshot")).unwrap();
    let data = work_dir().join("phone");
    let _ = std::fs::remove_dir_all(&data);
    eprintln!("-- LoroBackend: phone receives the library (unsigned values) --");
    runtime().block_on(async {
        let base = mark();
        let phone =
            LoroBackend::new(LoroNetwork::new(), dev(20), ManualClock::new(1)).with_data_dir(&data);
        let lib = phone.open_library(&cap()).await.unwrap();
        lib.import_state(&snap).await.unwrap();
        report("phone replica after receiving everything", base, n);
        let page = lib
            .photos_by_time(TimeRange::ALL, Page::first(200))
            .await
            .unwrap();
        assert_eq!(page.len(), 200.min(n));
        let all = lib
            .photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap();
        report("  ... while holding photos_by_time(all)", base, n);
        drop(all);
        drop(lib);
        drop(phone);
        // Let the flush thread notice the library is gone.
        std::thread::sleep(std::time::Duration::from_millis(1_500));
    });
    eprintln!();
}

/// The phone app starts: the replica is opened from disk.
fn open(n: usize) {
    let data = work_dir().join("phone");
    eprintln!("-- LoroBackend: phone opens its replica from disk --");
    runtime().block_on(async {
        let base = mark();
        let phone =
            LoroBackend::new(LoroNetwork::new(), dev(20), ManualClock::new(1)).with_data_dir(&data);
        let lib = phone.open_library(&cap()).await.unwrap();
        report("phone replica opened from disk", base, n);
        let page = lib
            .photos_by_time(TimeRange::ALL, Page::first(200))
            .await
            .unwrap();
        assert_eq!(page.len(), 200.min(n));
    });
    eprintln!();
}
