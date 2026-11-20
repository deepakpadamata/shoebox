//! The iCloud Photos export reader against `fixtures/exports/icloud` (`cargo xtask exports`) and
//! its `truth.json`.

use shoebox_media::icloud::{plan, HIDDEN_ALBUM};
use shoebox_media::takeout::{resolve_taken_at_cut, Entry, Left, Rule, Source};
use shoebox_media::TakenAtSource;
use std::path::{Path, PathBuf};

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
            out.push((
                Entry {
                    part,
                    path: rel,
                    size: e.metadata().unwrap().len(),
                    crc32: None,
                },
                path,
            ));
        }
    }
}

fn fixture() -> (PathBuf, Vec<(Entry, PathBuf)>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/exports/icloud");
    let mut files = vec![];
    walk(&dir, &dir.join("iCloud Photos"), 0, &mut files);
    (dir, files)
}

#[test]
fn checked_in_export_matches_its_truth() {
    let (dir, files) = fixture();
    let truth: Vec<xtask::exports::IcloudCase> =
        serde_json::from_slice(&std::fs::read(dir.join("truth.json")).unwrap()).unwrap();
    let entries: Vec<Entry> = files.iter().map(|(e, _)| e.clone()).collect();
    let mut asked = vec![];
    let p = plan(&entries, |i| {
        asked.push(i);
        std::fs::read(&files[i].1).ok()
    })
    .expect("an iCloud export");
    assert_eq!(p.source, Source::Icloud);
    assert!(asked.iter().all(|&i| entries[i].path.ends_with(".csv")));
    let at = |path: &str| entries.iter().position(|e| e.path == path);
    let mut wrong: Vec<String> = vec![];
    let mut check = |ok: bool, path: &str, what: String| {
        if !ok {
            wrong.push(format!("{path}: {what}"));
        }
    };
    for c in &truth {
        let name = c.path.rsplit('/').next().unwrap();
        let Some(i) = at(&c.path) else {
            check(c.missing, &c.path, "not in the export".into());
            check(
                p.missing.iter().any(|m| m == name),
                &c.path,
                format!("not reported missing: {:?}", p.missing),
            );
            continue;
        };
        let Some(item) = p.item_of(i).map(|n| &p.items[n]) else {
            check(
                c.deleted && p.left.contains(&(i, Left::Trashed)),
                &c.path,
                "in no item".into(),
            );
            continue;
        };
        check(!c.deleted, &c.path, "deleted but planned".into());
        check(
            item.sidecar
                .map(|(s, r)| (entries[s].path.ends_with("Photo Details.csv"), r))
                == Some((true, Rule::Listed)),
            &c.path,
            format!("row {:?}", item.sidecar),
        );
        check(
            item.meta.favorite == c.favorite,
            &c.path,
            format!("favourite {}", item.meta.favorite),
        );
        let mut albums: Vec<&str> = item
            .albums
            .iter()
            .map(|&a| p.albums[a].title.as_str())
            .collect();
        albums.sort_unstable();
        let mut want: Vec<&str> = c.albums.iter().map(String::as_str).collect();
        want.sort_unstable();
        check(
            albums == want,
            &c.path,
            format!("albums {albums:?}, want {want:?}"),
        );
        check(
            albums.contains(&HIDDEN_ALBUM) == c.hidden,
            &c.path,
            "hidden album".into(),
        );
        let live = item
            .live_photo_of
            .map(|n| entries[p.items[n].entries[0]].path.clone());
        check(
            live == c.live_photo_of,
            &c.path,
            format!("live photo of {live:?}"),
        );
        // The date the import gives it, as `apply_export` decides it.
        if let Some(want) = c.taken_at_ms {
            let r = shoebox_media::analyze(&files[i].1).unwrap();
            let file = (r.taken_at_source == TakenAtSource::Exif).then_some(r.taken_at.0);
            let got = resolve_taken_at_cut(file, item.meta.taken_at_ms, Rule::Listed.date_cut_ms());
            check(
                got == Some(want),
                &c.path,
                format!("date {got:?}, want {want}"),
            );
        }
    }
    let mut titles: Vec<&str> = p.albums.iter().map(|a| a.title.as_str()).collect();
    titles.sort_unstable();
    check(
        titles == ["Hidden (from iCloud)", "Trip 2015"],
        "plan",
        format!("albums {titles:?}"),
    );
    check(
        p.unmatched.is_empty(),
        "plan",
        format!("unmatched {:?}", p.unmatched),
    );
    check(
        p.ambiguous.is_empty(),
        "plan",
        format!("ambiguous {:?}", p.ambiguous),
    );
    for (i, why) in &p.left {
        let path = &entries[*i].path;
        let ok = match why {
            Left::Trashed => true,
            Left::NotPhotos => path.contains("/Memories/"),
            _ => false,
        };
        check(ok, path, format!("left as {why:?}"));
    }
    assert!(
        wrong.is_empty(),
        "{} mismatches:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_takeout_is_not_an_icloud_export() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/exports/google-takeout");
    let mut files = vec![];
    walk(&dir, &dir.join("Takeout"), 0, &mut files);
    let entries: Vec<Entry> = files.iter().map(|(e, _)| e.clone()).collect();
    assert!(plan(&entries, |i| std::fs::read(&files[i].1).ok()).is_none());
}

/// Parts that each have their own root (as a split export may) still find their rows, and a part
/// whose root holds no details file still counts its `Photos` folder.
#[test]
fn parts_with_their_own_roots() {
    let e = |part, path: &str| Entry {
        part,
        path: path.into(),
        size: 10,
        crc32: None,
    };
    let entries = vec![
        e(0, "iCloud Photos Part 1 of 2/Photos/Photo Details.csv"),
        e(0, "iCloud Photos Part 1 of 2/Photos/IMG_1.JPG"),
        e(1, "iCloud Photos Part 2 of 2/Photos/IMG_2.HEIC"),
        e(1, "iCloud Photos Part 2 of 2/Photos/IMG_2.MOV"),
        e(1, "iCloud Photos Part 2 of 2/Photos/IMG_3.JPG"),
        e(1, "iCloud Photos Part 2 of 2/Albums/Kids.csv"),
        e(1, "iCloud Photos Part 2 of 2/Favorites.csv"),
        e(1, "iCloud Photos Part 2 of 2/Shared Albums/AlbumInfo.json"),
    ];
    let csv = |i: usize| -> Option<Vec<u8>> {
        Some(match i {
            0 => b"imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate\n\
IMG_1.JPG,A,no,no,no,\"Monday January 26,2015 12:53 PM GMT\",0,x\n\
IMG_2.HEIC,A,no,no,no,\"Monday January 26,2015 12:54 PM GMT\",0,x\n\
IMG_2.MOV,A,no,no,no,\"Monday January 26,2015 12:54 PM GMT\",0,x\n\
IMG_4.JPG,A,no,no,no,\"Monday January 26,2015 12:55 PM GMT\",0,x\n\
IMG_5.JPG,A,no,no,yes,\"Monday January 26,2015 12:56 PM GMT\",0,x\n"
                .to_vec(),
            5 => b"imgName\nIMG_2.HEIC\n".to_vec(),
            6 => b"Images\nIMG_1.JPG\n".to_vec(),
            _ => return None,
        })
    };
    let p = plan(&entries, csv).unwrap();
    let item = |i| &p.items[p.item_of(i).unwrap()];
    assert_eq!(item(1).meta.taken_at_ms, Some(1_422_276_780_000));
    assert!(item(1).meta.favorite);
    assert_eq!(item(2).sidecar.map(|s| s.0), Some(0));
    assert_eq!(item(3).live_photo_of, p.item_of(2));
    assert_eq!(item(3).albums, vec![0]);
    assert_eq!(p.albums[0].title, "Kids");
    // No row for IMG_3; IMG_4 has a row and no file; IMG_5 was deleted and is not missed.
    assert_eq!(p.unmatched, vec![4]);
    assert_eq!(p.missing, vec!["IMG_4.JPG".to_owned()]);
    assert_eq!(p.left, vec![(7, Left::NotPhotos)]);
}
