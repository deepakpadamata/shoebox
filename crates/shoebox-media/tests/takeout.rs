//! I1-05: the Takeout reader over hand-written exports, one per quirk in
//! `docs/notes/takeout-format.md`, and (when `SHOEBOX_TAKEOUT_SAMPLES` names the folder) over the
//! public samples, which stay outside the repository.

use shoebox_media::takeout::{plan, Entry, Left, Rule, TakeoutPlan};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ROOT: &str = "Takeout/Google\u{a0}Photos";

fn sidecar(title: &str, taken: i64) -> String {
    format!(
        r#"{{"title":"{title}","description":"","imageViews":"1",
            "creationTime":{{"timestamp":"1700000000","formatted":"x"}},
            "photoTakenTime":{{"timestamp":"{taken}","formatted":"x"}},
            "geoData":{{"latitude":0.0,"longitude":0.0,"altitude":0.0}},
            "geoDataExif":{{"latitude":12.5,"longitude":76.25,"altitude":0.0}}}}"#
    )
}

fn album(title: &str) -> String {
    format!(
        r#"{{"title":"{title}","description":"","access":"protected","date":{{"timestamp":"1700000000"}}}}"#
    )
}

/// An export from `(path under the root, content)`; media get their content's length as size.
struct Export {
    entries: Vec<Entry>,
    json: BTreeMap<usize, Vec<u8>>,
}

fn export(files: &[(&str, &str)]) -> Export {
    let mut entries = vec![];
    let mut json = BTreeMap::new();
    for (i, (path, content)) in files.iter().enumerate() {
        let path = if path.starts_with("Takeout/") {
            path.to_string()
        } else {
            format!("{ROOT}/{path}")
        };
        if path.ends_with(".json") {
            json.insert(i, content.as_bytes().to_vec());
        }
        entries.push(Entry {
            part: i % 3,
            path,
            size: content.len() as u64,
            crc32: Some(crc(content.as_bytes())),
        });
    }
    Export { entries, json }
}

fn crc(bytes: &[u8]) -> u32 {
    // Any stable function will do for the reader; it only compares them.
    bytes.iter().fold(0x811c_9dc5u32, |h, b| {
        (h ^ *b as u32).wrapping_mul(0x0100_0193)
    })
}

fn run(x: &Export) -> TakeoutPlan {
    plan(&x.entries, |i| x.json.get(&i).cloned())
}

fn index(x: &Export, path: &str) -> usize {
    x.entries
        .iter()
        .position(|e| e.path.ends_with(path))
        .unwrap_or_else(|| panic!("no entry {path}"))
}

fn rule_of(x: &Export, p: &TakeoutPlan, media: &str) -> Option<(String, Rule)> {
    let item = &p.items[p.item_of(index(x, media)).expect("in an item")];
    item.sidecar
        .map(|(s, r)| (x.entries[s].path.rsplit('/').next().unwrap().to_owned(), r))
}

#[test]
fn every_naming_rule() {
    let long = "PXL_20230814_201154491.LONG_EXPOSURE-01.COVER";
    let emoji = "😀😃😄😁😆😅😂🤣🥲☺️😊😇🙂🙃😉😌😍🥰😘😗😙😚😋";
    let files = vec![
        ("Photos from 2023/a.jpg", "A"),
        (
            "Photos from 2023/a.jpg.json",
            &*sidecar("a.jpg", 1_600_000_000).leak(),
        ),
        ("Photos from 2023/b.jpg", "BB"),
        (
            "Photos from 2023/b.jpg.supplemental-metadata.json",
            sidecar("b.jpg", 1_600_000_001).leak(),
        ),
        ("Photos from 2023/DSC_0238.JPG", "C1"),
        (
            "Photos from 2023/DSC_0238.JPG.json",
            sidecar("DSC_0238.JPG", 1).leak(),
        ),
        ("Photos from 2023/DSC_0238(1).JPG", "C22"),
        (
            "Photos from 2023/DSC_0238.JPG(1).json",
            sidecar("DSC_0238.JPG", 2).leak(),
        ),
        ("Photos from 2023/IMG_9(2).jpg", "D"),
        (
            "Photos from 2023/IMG_9.jpg.supplemental-metadata(2).json",
            sidecar("IMG_9.jpg", 3).leak(),
        ),
        (&*format!("Photos from 2023/{long}.jpg").leak(), "E"),
        (
            &*format!("Photos from 2023/{long}..json").leak(),
            sidecar("x", 4).leak(),
        ),
        (&*format!("Photos from 2023/{emoji}😛.jpg").leak(), "F"),
        (
            &*format!("Photos from 2023/{emoji}.json").leak(),
            sidecar("y", 5).leak(),
        ),
        ("Photos from 2023/PXL_1.MP", "G"),
        (
            "Photos from 2023/PXL_1.MP.jpg.json",
            sidecar("PXL_1.MP.jpg", 6).leak(),
        ),
        ("Photos from 2023/odd name.jpg", "H"),
        (
            "Photos from 2023/something else.json",
            sidecar("odd name.jpg", 7).leak(),
        ),
        ("Photos from 2023/Upper.JPG", "I"),
        ("Photos from 2023/upper.jpg.json", sidecar("q", 8).leak()),
        ("Photos from 2023/IMG_8416.JPG", "J"),
        (
            "Photos from 2023/IMG_8416.JPG.json",
            sidecar("IMG_8416.JPG", 9).leak(),
        ),
        ("Photos from 2023/IMG_8416-modifié.JPG", "JJ"),
        ("Photos from 2023/IMG_1.HEIC", "K"),
        (
            "Photos from 2023/IMG_1.HEIC.json",
            sidecar("IMG_1.HEIC", 10).leak(),
        ),
        ("Photos from 2023/IMG_1.MP4", "KK"),
        ("Photos from 2023/lonely.png", "L"),
    ];
    let x = export(&files);
    let p = run(&x);
    let r = |m| rule_of(&x, &p, m).map(|(_, r)| r);
    assert_eq!(r("/a.jpg"), Some(Rule::Exact));
    assert_eq!(r("/b.jpg"), Some(Rule::Exact));
    assert_eq!(
        rule_of(&x, &p, "/DSC_0238.JPG").unwrap().0,
        "DSC_0238.JPG.json"
    );
    assert_eq!(
        rule_of(&x, &p, "/DSC_0238(1).JPG").unwrap(),
        ("DSC_0238.JPG(1).json".into(), Rule::MovedNumber)
    );
    assert_eq!(r("/IMG_9(2).jpg"), Some(Rule::MovedNumber));
    assert_eq!(r("COVER.jpg"), Some(Rule::Exact));
    assert_eq!(r("😛.jpg"), Some(Rule::Exact));
    assert_eq!(r("/PXL_1.MP"), Some(Rule::PixelMp));
    assert_eq!(r("/odd name.jpg"), Some(Rule::Title));
    assert_eq!(r("/Upper.JPG"), Some(Rule::IgnoringCase));
    assert_eq!(r("/IMG_8416-modifié.JPG"), Some(Rule::EditOf));
    assert_eq!(r("/IMG_1.MP4"), Some(Rule::LivePhoto));
    assert_eq!(r("/lonely.png"), None);
    assert_eq!(p.unmatched, vec![index(&x, "/lonely.png")]);
    assert!(p.ambiguous.is_empty());
    assert!(p.left.is_empty(), "{:?}", p.left);

    let item = |m| &p.items[p.item_of(index(&x, m)).unwrap()];
    assert_eq!(item("/a.jpg").meta.taken_at_ms, Some(1_600_000_000_000));
    // geoData is (0, 0): geoDataExif's place.
    assert_eq!(item("/a.jpg").meta.place, Some((12.5, 76.25)));
    let original = p.item_of(index(&x, "/IMG_8416.JPG"));
    assert_eq!(item("/IMG_8416-modifié.JPG").edit_of, original);
    assert_eq!(
        item("/IMG_1.MP4").live_photo_of,
        p.item_of(index(&x, "/IMG_1.HEIC"))
    );
}

#[test]
fn albums_copies_trash_and_archive() {
    let files = vec![
        ("Photos from 2019/x.jpg", "X"),
        ("Photos from 2019/x.jpg.json", sidecar("x.jpg", 100).leak()),
        ("Kerala trip/x.jpg", "X"),
        ("Kerala trip/x.jpg.json", sidecar("x.jpg", 100).leak()),
        ("Kerala trip/métadonnées.json", album("Kerala trip").leak()),
        // A cut and escaped folder name: the title comes from the metadata file.
        ("Album test 6-10-23/x.jpg", "X"),
        (
            "Album test 6-10-23/x.jpg.json",
            r#"{"title":"x.jpg","favorited":true,"photoTakenTime":{"timestamp":"100"}}"#,
        ),
        (
            "Album test 6-10-23/metadata.json",
            album("Album test 6/10/23").leak(),
        ),
        // An album with no title: its folder's name.
        ("Sans titre(9)/y.jpg", "Y"),
        ("Sans titre(9)/y.jpg.json", sidecar("y.jpg", 200).leak()),
        ("Sans titre(9)/métadonnées.json", album("").leak()),
        // Same name, other bytes: another photo.
        ("Photos from 2020/x.jpg", "XX"),
        ("Photos from 2020/x.jpg.json", sidecar("x.jpg", 300).leak()),
        ("Corbeille/z.jpg", "Z"),
        ("Corbeille/z.jpg.json", sidecar("z.jpg", 400).leak()),
        ("Photos from 2021/t.jpg", "T"),
        (
            "Photos from 2021/t.jpg.json",
            r#"{"title":"t.jpg","trashed":true,"photoTakenTime":{"timestamp":"5"}}"#,
        ),
        ("Archive/w.jpg", "W"),
        ("Archive/w.jpg.json", sidecar("w.jpg", 500).leak()),
        ("Takeout/archive_browser.html", "<html>"),
        ("Takeout/Drive/report.pdf", "%PDF"),
        ("Photos from 2019/broken.json", "{not json"),
        (
            "Photos from 2019/stray.jpg.json",
            sidecar("stray.jpg", 600).leak(),
        ),
    ];
    let x = export(&files);
    let p = run(&x);

    let mut titles: Vec<&str> = p.albums.iter().map(|a| a.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(
        titles,
        ["Album test 6/10/23", "Kerala trip", "Sans titre(9)"]
    );
    let x_item = &p.items[p.item_of(index(&x, "2019/x.jpg")).unwrap()];
    assert_eq!(
        x_item.entries.len(),
        3,
        "the year folder's copy and two album copies"
    );
    assert_eq!(
        x_item.entries[0],
        index(&x, "2019/x.jpg"),
        "read from the year folder"
    );
    let mut in_albums: Vec<&str> = x_item
        .albums
        .iter()
        .map(|&a| p.albums[a].title.as_str())
        .collect();
    in_albums.sort_unstable();
    assert_eq!(in_albums, ["Album test 6/10/23", "Kerala trip"]);
    assert!(x_item.meta.favorite, "a favourite in any copy's sidecar");
    let other = p.item_of(index(&x, "2020/x.jpg")).unwrap();
    assert_ne!(Some(other), p.item_of(index(&x, "2019/x.jpg")));
    assert!(
        p.items[p.item_of(index(&x, "Archive/w.jpg")).unwrap()]
            .meta
            .archived
    );
    assert_eq!(p.item_of(index(&x, "Corbeille/z.jpg")), None);
    assert_eq!(p.item_of(index(&x, "2021/t.jpg")), None);

    let left: BTreeMap<usize, Left> = p.left.iter().cloned().collect();
    assert_eq!(left[&index(&x, "Corbeille/z.jpg")], Left::Trashed);
    assert_eq!(left[&index(&x, "2021/t.jpg")], Left::Trashed);
    assert_eq!(left[&index(&x, "archive_browser.html")], Left::NotPhotos);
    assert_eq!(left[&index(&x, "report.pdf")], Left::NotPhotos);
    assert!(matches!(
        left[&index(&x, "broken.json")],
        Left::UnreadableJson(_)
    ));
    assert_eq!(left[&index(&x, "stray.jpg.json")], Left::UnusedSidecar);
    assert_eq!(p.items.len(), 4);
}

#[test]
fn ambiguous_claims_are_reported_not_guessed() {
    // Two long names cut to the same sidecar name, and a title that names neither.
    let a = "PXL_20230814_201154491.LONG_EXPOSURE-01.COVER.jpg";
    let b = "PXL_20230814_201154491.LONG_EXPOSURE-01.COVER.jpeg";
    let s = "PXL_20230814_201154491.LONG_EXPOSURE-01.COVER..json";
    let files = vec![
        (&*format!("Photos from 2023/{a}").leak(), "A"),
        (&*format!("Photos from 2023/{b}").leak(), "B"),
        (
            &*format!("Photos from 2023/{s}").leak(),
            sidecar("other.jpg", 1).leak(),
        ),
    ];
    let x = export(&files);
    let p = run(&x);
    assert_eq!(p.ambiguous.len(), 2);
    assert!(p.items.iter().all(|i| i.sidecar.is_none()));

    // The same, with the title naming one of them: that one gets it.
    let files = vec![
        (&*format!("Photos from 2023/{a}").leak(), "A"),
        (&*format!("Photos from 2023/{b}").leak(), "B"),
        (
            &*format!("Photos from 2023/{s}").leak(),
            sidecar(a, 1).leak(),
        ),
    ];
    let x = export(&files);
    let p = run(&x);
    assert!(p.ambiguous.is_empty());
    assert_eq!(rule_of(&x, &p, a).map(|(_, r)| r), Some(Rule::Exact));
    assert_eq!(rule_of(&x, &p, b), None);
}

#[test]
fn without_crc_copies_join_only_when_their_dates_agree() {
    let mut x = export(&[
        ("Photos from 2019/IMG_0001.JPG", "same size A"),
        (
            "Photos from 2019/IMG_0001.JPG.json",
            sidecar("IMG_0001.JPG", 1).leak(),
        ),
        ("Photos from 2021/IMG_0001.JPG", "same size B"),
        (
            "Photos from 2021/IMG_0001.JPG.json",
            sidecar("IMG_0001.JPG", 2).leak(),
        ),
        ("Trip/IMG_0001.JPG", "same size A"),
        ("Trip/IMG_0001.JPG.json", sidecar("IMG_0001.JPG", 1).leak()),
        ("Trip/metadata.json", album("Trip").leak()),
    ]);
    for e in &mut x.entries {
        e.crc32 = None;
    }
    let p = run(&x);
    assert_eq!(p.items.len(), 2);
    let a = &p.items[p.item_of(index(&x, "2019/IMG_0001.JPG")).unwrap()];
    assert_eq!(a.entries.len(), 2);
    assert_eq!(a.albums.len(), 1);
}

#[test]
fn a_hundred_thousand_entries_plan_quickly() {
    let n = 100_000;
    let mut entries = Vec::with_capacity(2 * n + 1);
    let mut json = vec![];
    for i in 0..n {
        let dir = format!("{ROOT}/Photos from {}", 2000 + i % 25);
        entries.push(Entry {
            part: i % 10,
            path: format!("{dir}/IMG_{i:06}.jpg"),
            size: 1000 + i as u64,
            crc32: Some(i as u32),
        });
        json.push(sidecar(&format!("IMG_{i:06}.jpg"), 1_600_000_000 + i as i64).into_bytes());
        entries.push(Entry {
            part: (i + 3) % 10,
            path: format!("{dir}/IMG_{i:06}.jpg.supplemental-metadata.json"),
            size: 600,
            crc32: Some(0),
        });
    }
    let started = std::time::Instant::now();
    let p = plan(&entries, |i| Some(json[i / 2].clone()));
    let took = started.elapsed();
    assert_eq!(p.items.len(), n);
    assert!(p.unmatched.is_empty());
    eprintln!("100k entries planned in {took:?}");
    // The acceptance number is for a release build on the Mac (10 s); a debug build gets more room.
    assert!(
        took.as_secs() < if cfg!(debug_assertions) { 60 } else { 10 },
        "{took:?}"
    );
}

// ---------- the public samples (not in the repository) ----------

fn samples() -> Option<PathBuf> {
    std::env::var_os("SHOEBOX_TAKEOUT_SAMPLES").map(PathBuf::from)
}

fn walk(root: &Path, dir: &Path, part: usize, out: &mut Vec<(Entry, PathBuf)>) {
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    names.sort_by_key(|e| e.file_name());
    for e in names {
        let path = e.path();
        if path.is_dir() {
            walk(root, &path, part, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let size = e.metadata().unwrap().len();
            out.push((
                Entry {
                    part,
                    path: rel,
                    size,
                    crc32: None,
                },
                path,
            ));
        }
    }
}

/// immich-go's three small Takeouts (`Takeout1`…`3`), read as one export of three parts.
#[test]
fn immich_go_takeouts() {
    let Some(root) = samples() else {
        eprintln!("SHOEBOX_TAKEOUT_SAMPLES not set: skipped");
        return;
    };
    let mut files = vec![];
    for (part, name) in ["Takeout1", "Takeout2", "Takeout3"].iter().enumerate() {
        let dir = root.join("immich-go").join(name);
        walk(&dir, &dir, part, &mut files);
    }
    let entries: Vec<Entry> = files.iter().map(|(e, _)| e.clone()).collect();
    let p = plan(&entries, |i| std::fs::read(&files[i].1).ok());
    for (i, item) in p.items.iter().enumerate() {
        let e = &entries[item.entries[0]];
        eprintln!(
            "{i:2} {} copies={} sidecar={:?} albums={:?} taken={:?}",
            e.path,
            item.entries.len(),
            item.sidecar
                .map(|(s, r)| (entries[s].path.rsplit('/').next().unwrap().to_owned(), r)),
            item.albums
                .iter()
                .map(|a| &p.albums[*a].title)
                .collect::<Vec<_>>(),
            item.meta.taken_at_ms,
        );
    }
    eprintln!(
        "albums {:?}\nunmatched {:?}\nambiguous {:?}\nleft {:?}",
        p.albums, p.unmatched, p.ambiguous, p.left
    );
    assert!(p.unmatched.is_empty());
    assert!(p.ambiguous.is_empty());
    assert!(p.left.is_empty());
    let titles: Vec<&str> = p.albums.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["Album test 6/10/23", "Sans titre(9)"]);
}

/// The file listing of a real three-part Takeout from May 2024 (names and sizes, no contents):
/// every `.json` is taken as a sidecar with no title, except the albums' metadata files.
#[test]
fn may_2024_listing() {
    let Some(root) = samples() else {
        eprintln!("SHOEBOX_TAKEOUT_SAMPLES not set: skipped");
        return;
    };
    let text = std::fs::read_to_string(root.join("immich-go/small.lst")).unwrap();
    let mut entries = vec![];
    let mut part = 0;
    for line in text.lines() {
        if line.starts_with("takeout-") && line.ends_with(':') {
            part = line[..line.len() - 5]
                .rsplit('-')
                .next()
                .unwrap()
                .parse::<usize>()
                .unwrap()
                - 1;
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(start) = line.find("Takeout/") else {
            continue;
        };
        let size: u64 = fields[0].parse().unwrap();
        entries.push(Entry {
            part,
            path: line[start..].to_owned(),
            size,
            crc32: None,
        });
    }
    assert_eq!(entries.len(), 968);
    let p = plan(&entries, |i| {
        let name = entries[i].path.rsplit('/').next().unwrap();
        Some(if name == "métadonnées.json" {
            album("").into_bytes()
        } else {
            br#"{"photoTakenTime":{"timestamp":"0"}}"#.to_vec()
        })
    });
    let mut by_rule: BTreeMap<String, usize> = BTreeMap::new();
    for item in &p.items {
        *by_rule
            .entry(format!("{:?}", item.sidecar.map(|(_, r)| r)))
            .or_default() += 1;
    }
    eprintln!("items {} by rule {by_rule:?}", p.items.len());
    eprintln!(
        "unmatched {:?}",
        p.unmatched
            .iter()
            .map(|&i| &entries[i].path)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "ambiguous {:?}",
        p.ambiguous
            .iter()
            .map(|&(i, _)| &entries[i].path)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "left {:?}",
        p.left
            .iter()
            .map(|(i, w)| (&entries[*i].path, w))
            .collect::<Vec<_>>()
    );
    assert!(p.unmatched.is_empty());
    assert!(p.ambiguous.is_empty());
    assert_eq!(p.albums.len(), 3);
}

// ---------- the fixture export (`cargo xtask fixtures --preset takeout`) ----------

fn fixture_export(dir: &Path) -> (Vec<PathBuf>, Vec<xtask::takeout::Truth>) {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let summary = xtask::takeout::generate(dir, &samples, 7, 3).unwrap();
    let truth = serde_json::from_slice(&std::fs::read(&summary.truth).unwrap()).unwrap();
    (summary.parts, truth)
}

#[test]
fn fixture_export_matches_its_truth() {
    use shoebox_media::archive::ArchiveSet;
    let dir = tempfile::tempdir().unwrap();
    let (parts, truth) = fixture_export(dir.path());
    let (set, errors) = ArchiveSet::open(&parts);
    assert!(errors.is_empty(), "{errors:?}");
    let entries = set.takeout_entries();
    let json: std::collections::BTreeSet<usize> = (0..entries.len())
        .filter(|&i| entries[i].path.ends_with(".json"))
        .collect();
    let bytes = set.read_small(&json);
    let p = plan(&entries, |i| bytes.get(&i).cloned());

    let at = |path: &str| {
        entries
            .iter()
            .position(|e| e.path == path)
            .unwrap_or_else(|| panic!("{path}"))
    };
    let path_of = |i: usize| entries[i].path.clone();
    let mut crossing = 0;
    for t in &truth {
        let i = at(&t.path);
        assert_eq!(entries[i].part, t.part, "{}", t.path);
        let Some(item) = p.item_of(i).map(|n| &p.items[n]) else {
            assert!(t.trashed, "{} is in no item", t.path);
            continue;
        };
        assert!(!t.trashed, "{} is trashed but planned", t.path);
        if let Some(copy_of) = &t.copy_of {
            assert_eq!(path_of(item.entries[0]), *copy_of, "{}", t.path);
        } else {
            assert_eq!(item.entries[0], i, "{} is read first", t.path);
            assert_eq!(
                item.sidecar.map(|(s, _)| path_of(s)),
                t.sidecar,
                "{}",
                t.path
            );
            if let Some(s) = &t.sidecar {
                crossing += usize::from(entries[at(s)].part != t.part);
            }
        }
        let mut albums: Vec<String> = item
            .albums
            .iter()
            .map(|&a| p.albums[a].title.clone())
            .collect();
        albums.sort();
        let mut want = t.albums.clone();
        want.sort();
        assert_eq!(albums, want, "{}", t.path);
        assert_eq!(item.meta.favorite, t.favorite, "{}", t.path);
        assert_eq!(item.meta.archived, t.archived, "{}", t.path);
        let first_of = |n: Option<usize>| n.map(|n| path_of(p.items[n].entries[0]));
        assert_eq!(first_of(item.edit_of), t.edit_of, "{}", t.path);
        assert_eq!(first_of(item.live_photo_of), t.live_photo_of, "{}", t.path);
    }
    assert!(
        crossing >= 5,
        "only {crossing} sidecars are in another part than their file"
    );
    assert!(
        p.unmatched.is_empty() && p.ambiguous.is_empty(),
        "{:?} {:?}",
        p.unmatched,
        p.ambiguous
    );
    let left: Vec<&Left> = p.left.iter().map(|(_, l)| l).collect();
    assert!(
        left.iter()
            .all(|l| matches!(l, Left::Trashed | Left::NotPhotos)),
        "{left:?}"
    );
}
