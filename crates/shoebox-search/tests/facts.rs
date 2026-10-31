//! The facts index (S1-03): filters, keyword ranking, updates and persistence.

use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::{AlbumId, BlobRef, ExifSummary, Gps, PhotoId, PhotoRecord};
use shoebox_search::calendar::{Date, Zone};
use shoebox_search::facts::{decode, encode, FactsIndex, Field, PhotoFacts};
use shoebox_search::query::{parse, ParseContext};
use std::time::Instant;

fn blob(tag: &str, mime: &str) -> BlobRef {
    BlobRef {
        hash: Blake3Hash::of_bytes(tag.as_bytes()),
        size: 1,
        mime: mime.into(),
    }
}

struct P {
    name: &'static str,
    date: (i32, u8, u8),
    mime: &'static str,
    camera: Option<(&'static str, &'static str)>,
    lens: Option<&'static str>,
    size: (u32, u32),
    gps: Option<(f64, f64)>,
    fav: bool,
}

const fn p(name: &'static str, date: (i32, u8, u8)) -> P {
    P {
        name,
        date,
        mime: "image/jpeg",
        camera: Some(("Canon", "Canon EOS R5")),
        lens: None,
        size: (4000, 3000),
        gps: None,
        fav: false,
    }
}

fn id(name: &str) -> PhotoId {
    Blake3Hash::of_bytes(name.as_bytes())
}

fn record(p: &P) -> PhotoRecord {
    PhotoRecord {
        id: id(p.name),
        original: blob(p.name, p.mime),
        preview: blob(&format!("{}-p", p.name), "image/jpeg"),
        thumb: blob(&format!("{}-t", p.name), "image/jpeg"),
        taken_at: Zone::UTC.start_of(Date::new(p.date.0, p.date.1, p.date.2)),
        exif: ExifSummary {
            camera_make: p.camera.map(|c| c.0.to_string()),
            camera_model: p.camera.map(|c| c.1.to_string()),
            lens: p.lens.map(str::to_string),
            width: p.size.0,
            height: p.size.1,
            gps: p.gps.map(|(la, lo)| Gps {
                lat_e7: (la * 1e7) as i32,
                lon_e7: (lo * 1e7) as i32,
            }),
        },
        imported_by: EndpointId([1; 32]),
        favorite: Lww::new(p.fav, Hlc::GENESIS),
        deleted_at: Lww::new(None, Hlc::GENESIS),
    }
}

fn library() -> FactsIndex {
    let photos = [
        p("a", (2019, 3, 2)),
        P {
            fav: true,
            ..p("b", (2019, 7, 14))
        },
        P {
            mime: "video/mp4",
            ..p("c", (2019, 12, 25))
        },
        P {
            camera: Some(("Apple", "iPhone 15 Pro")),
            lens: Some("iPhone 15 Pro front camera 2.69mm f/1.9"),
            ..p("d", (2021, 3, 6))
        },
        P {
            camera: None,
            mime: "image/png",
            size: (1170, 2532),
            ..p("e", (2021, 3, 7))
        },
        P {
            camera: None,
            size: (2532, 1170),
            ..p("f", (2022, 1, 1))
        },
        P {
            mime: "image/x-adobe-dng",
            ..p("g", (2022, 6, 1))
        },
        // Southern hemisphere: January is summer there.
        P {
            gps: Some((-33.9, 18.4)),
            ..p("h", (2023, 1, 10))
        },
        P {
            gps: Some((38.7, -9.1)),
            ..p("i", (2023, 7, 10))
        },
    ];
    let mut ix = FactsIndex::new();
    for ph in &photos {
        ix.upsert(PhotoFacts::of(&record(ph)));
    }
    ix.set_album(AlbumId([1; 16]), "Lisbon trip", [id("i"), id("a")]);
    ix.set_album(AlbumId([2; 16]), "Wedding", [id("b")]);
    ix.set_extra(
        &id("i"),
        Field::Place,
        Some("Lisbon\nLisbon District\nPortugal".into()),
    );
    ix.set_extra(
        &id("h"),
        Field::Place,
        Some("Cape Town\nWestern Cape\nSouth Africa".into()),
    );
    ix.set_extra(
        &id("a"),
        Field::Ocr,
        Some("TOTAL 12.50 receipt thank you".into()),
    );
    ix
}

fn cx() -> ParseContext {
    ParseContext {
        now: Zone::UTC.start_of(Date::new(2026, 10, 30)),
        zone: Zone::UTC,
        southern: false,
    }
}

fn names(ix: &FactsIndex, ids: &[PhotoId]) -> String {
    let all = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let _ = ix;
    ids.iter()
        .map(|i| *all.iter().find(|n| id(n) == *i).unwrap_or(&"?"))
        .collect::<Vec<_>>()
        .join(",")
}

fn run(ix: &FactsIndex, q: &str) -> String {
    let v = ix.vocabulary();
    let q = parse(q, &v, &cx());
    let hits = ix.search(&q, Zone::UTC, false, 0, 100);
    names(ix, &hits.ids)
}

#[test]
fn live_photos_are_the_ones_with_a_video() {
    let mut ix = library();
    assert_eq!(run(&ix, "live photos"), "", "none has a video yet");
    let b = P {
        fav: true,
        ..p("b", (2019, 7, 14))
    };
    assert!(ix.upsert(PhotoFacts::of(&record(&b)).with_live(true)));
    assert_eq!(run(&ix, "live photos"), "b");
    assert_eq!(run(&ix, "live photos favourites"), "b");
    assert_eq!(
        run(&ix, "photos 2019"),
        "b,a",
        "a Live Photo is still a photo"
    );
    // It stops being one when the video goes.
    assert!(ix.upsert(PhotoFacts::of(&record(&b))));
    assert_eq!(run(&ix, "live photos"), "");
}

#[test]
fn filters() {
    let ix = library();
    assert_eq!(run(&ix, ""), "i,h,g,f,e,d,c,b,a");
    assert_eq!(run(&ix, "2019"), "c,b,a");
    assert_eq!(run(&ix, "march"), "e,d,a");
    assert_eq!(run(&ix, "videos"), "c");
    assert_eq!(run(&ix, "photos 2019"), "b,a");
    assert_eq!(run(&ix, "favourites"), "b");
    assert_eq!(run(&ix, "christmas"), "c");
    assert_eq!(run(&ix, "screenshots"), "f,e");
    assert_eq!(run(&ix, "selfies"), "d");
    assert_eq!(run(&ix, "raw"), "g");
    assert_eq!(run(&ix, "canon eos r5 2022"), "g");
    assert_eq!(run(&ix, "iphone 15 pro"), "d");
    assert_eq!(run(&ix, "wedding"), "b");
    assert_eq!(run(&ix, "lisbon trip"), "i,a");
    assert_eq!(run(&ix, "portugal"), "i");
    assert_eq!(run(&ix, "summer"), "i,h,g,b");
    assert_eq!(run(&ix, "weekends 2021"), "e,d");
}

#[test]
fn keywords_rank_matches() {
    let ix = library();
    assert_eq!(run(&ix, "receipt"), "a");
    // "Lisbon" as a word: the place beats the album-only member.
    assert_eq!(run(&ix, "lisboa receipt"), "a");
    assert_eq!(run(&ix, "nothing like this"), "");
}

#[test]
fn deleted_and_removed_photos_are_not_found() {
    let mut ix = library();
    let mut rec = record(&p("a", (2019, 3, 2)));
    rec.deleted_at = Lww::new(Some(Timestamp(1)), Hlc::GENESIS);
    assert!(ix.upsert(PhotoFacts::of(&rec)));
    assert_eq!(run(&ix, "2019"), "c,b");
    assert_eq!(run(&ix, "receipt"), "");
    ix.remove(&id("b"));
    assert_eq!(run(&ix, "2019"), "c");
    assert!(
        !ix.upsert(PhotoFacts::of(&rec)),
        "unchanged facts are a no-op"
    );
}

#[test]
fn favouriting_moves_a_photo_into_favourites() {
    let mut ix = library();
    let mut rec = record(&p("c", (2019, 12, 25)));
    rec.original.mime = "video/mp4".into();
    rec.favorite = Lww::new(true, Hlc::GENESIS);
    ix.upsert(PhotoFacts::of(&rec));
    assert_eq!(run(&ix, "favourites"), "c,b");
}

#[test]
fn paging() {
    let ix = library();
    let q = parse("", &ix.vocabulary(), &cx());
    let a = ix.search(&q, Zone::UTC, false, 0, 4);
    assert_eq!((a.total, a.next), (9, Some(4)));
    let b = ix.search(&q, Zone::UTC, false, 4, 4);
    let c = ix.search(&q, Zone::UTC, false, 8, 4);
    assert_eq!(c.next, None);
    let all: Vec<_> = [a.ids, b.ids, c.ids].concat();
    assert_eq!(names(&ix, &all), "i,h,g,f,e,d,c,b,a");
}

#[test]
fn persists_and_rejects_damage() {
    let ix = library();
    let bytes = encode(&ix.snapshot());
    let back = FactsIndex::from_snapshot(decode(&bytes).expect("decodes"));
    assert_eq!(back.snapshot(), ix.snapshot());
    assert_eq!(
        run(&back, "lisbon trip receipt"),
        run(&ix, "lisbon trip receipt")
    );
    assert!(decode(&bytes[..bytes.len() - 1]).is_none(), "truncated");
    let mut flipped = bytes.clone();
    *flipped.last_mut().unwrap() ^= 1;
    assert!(decode(&flipped).is_none(), "corrupt");
    let mut other = bytes;
    other[4] = 99;
    assert!(decode(&other).is_none(), "other version");
}

#[test]
fn compaction_keeps_results() {
    let mut ix = library();
    for round in 0..30u32 {
        for n in ["a", "b", "g", "h", "i"] {
            let mut rec = record(&p(n, (2019, 3, 2)));
            rec.favorite = Lww::new(round % 2 == 0, Hlc::GENESIS);
            ix.upsert(PhotoFacts::of(&rec));
        }
    }
    assert_eq!(ix.len(), 9);
    assert_eq!(run(&ix, "receipt"), "a");
}

#[test]
fn hundred_thousand_photos() {
    let mut ix = FactsIndex::new();
    let start = Date::new(2005, 1, 1).days();
    let cams = ["Canon EOS R5", "iPhone 15 Pro", "Pixel 8", "X-T5"];
    for i in 0..100_000u32 {
        let name = format!("photo-{i}");
        let mut rec = record(&P {
            camera: Some(("", cams[i as usize % 4])),
            gps: (i % 3 == 0).then_some((10.0, 76.0)),
            fav: i % 50 == 0,
            ..p("x", (2005, 1, 1))
        });
        rec.id = id(&name);
        rec.taken_at = Timestamp((start + i64::from(i) / 12) * 86_400_000);
        ix.upsert(PhotoFacts::of(&rec));
        if i % 3 == 0 {
            ix.set_extra(&rec.id, Field::Place, Some("Kochi\nKerala\nIndia".into()));
        }
    }
    let bytes = encode(&ix.snapshot());
    assert!(bytes.len() < 15 << 20, "facts.idx is {} bytes", bytes.len());
    let v = ix.vocabulary();
    let t = Instant::now();
    let q = parse("kerala 2019 pixel 8", &v, &cx());
    let hits = ix.search(&q, Zone::UTC, false, 0, 200);
    let took = t.elapsed();
    assert!(hits.total > 0);
    // 50 ms on the Mac in release; unoptimised debug builds get headroom.
    let budget = if cfg!(debug_assertions) { 1000 } else { 50 };
    assert!(took.as_millis() < budget, "query took {took:?}");
}
