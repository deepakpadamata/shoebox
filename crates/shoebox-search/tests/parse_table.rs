//! Table tests for the query parser (S1-02): what each query becomes, as chips and leftover text.

use shoebox_core::ids::Timestamp;
use shoebox_search::calendar::{Date, Zone};
use shoebox_search::query::{parse, DateFilter, FilterKind, NameKind, ParseContext, Vocabulary};
use std::time::Instant;

fn vocab() -> Vocabulary {
    let mut v = Vocabulary::new();
    for (k, n) in [
        (NameKind::Place, "Paris"),
        (NameKind::Place, "Lisbon"),
        (NameKind::Place, "São Paulo"),
        (NameKind::Place, "Kerala"),
        (NameKind::Place, "New York"),
        (NameKind::Place, "New York City"),
        (NameKind::Place, "Portugal"),
        (NameKind::Person, "Paris Hilton"),
        (NameKind::Person, "Ana"),
        (NameKind::Person, "June"),
        (NameKind::Album, "Summer 2019"),
        (NameKind::Album, "Wedding"),
        (NameKind::Album, "Lisbon"),
        (NameKind::Camera, "Canon EOS R5"),
        (NameKind::Camera, "iPhone 15 Pro"),
        (NameKind::Lens, "RF 24-70mm F2.8"),
        (NameKind::Label, "dog"),
        (NameKind::Label, "beach"),
    ] {
        v.add(k, n);
    }
    v
}

fn cx() -> ParseContext {
    ParseContext {
        // Friday 2026-10-30, 12:00 UTC
        now: Timestamp(Date::new(2026, 10, 30).days() * 86_400_000 + 12 * 3_600_000),
        zone: Zone::UTC,
        southern: false,
    }
}

fn day(t: Option<Timestamp>) -> String {
    let d = Zone::UTC.date(t.unwrap());
    format!("{}-{:02}-{:02}", d.year, d.month, d.day)
}

/// One filter as a short string.
fn show(f: &FilterKind) -> String {
    match f {
        FilterKind::Date(DateFilter::Range(r)) => format!("{}..{}", day(r.start), day(r.end)),
        FilterKind::Date(DateFilter::Month(m)) => format!("month {m}"),
        FilterKind::Date(DateFilter::Season { season, year }) => match year {
            Some(y) => format!("{season:?} {y}"),
            None => format!("{season:?}"),
        },
        FilterKind::Date(DateFilter::Weekdays(m)) => format!("days {m:07b}"),
        FilterKind::Date(DateFilter::Holiday { holiday, year }) => match year {
            Some(y) => format!("{holiday:?} {y}"),
            None => format!("{holiday:?}"),
        },
        FilterKind::Media(k) => format!("{k:?}"),
        FilterKind::Favourite => "favourite".into(),
        FilterKind::Name { kind, name } => format!("{kind:?}:{name}"),
    }
}

/// (query, expected filters joined by " | ", expected leftover text)
const CASES: &[(&str, &str, &str)] = &[
    // years and ranges
    ("2019", "2019-01-01..2020-01-01", ""),
    ("photos 2019", "Photo | 2019-01-01..2020-01-01", ""),
    ("2018-2020", "2018-01-01..2021-01-01", ""),
    ("2018 – 2020", "2018-01-01..2021-01-01", ""),
    ("2020 to 2018", "2018-01-01..2021-01-01", ""),
    ("from 2018 until 2019", "2018-01-01..2020-01-01", ""),
    ("90s", "1990-01-01..2000-01-01", ""),
    ("the 1980s", "1980-01-01..1990-01-01", ""),
    ("2010s beach", "2010-01-01..2020-01-01", "beach"),
    ("12345", "", "12345"),
    ("room 1984", "1984-01-01..1985-01-01", "room"),
    // months
    ("March", "month 3", ""),
    ("march 2021", "2021-03-01..2021-04-01", ""),
    ("December of 2020", "2020-12-01..2021-01-01", ""),
    ("dec 2020", "2020-12-01..2021-01-01", ""),
    ("sept in 2015", "2015-09-01..2015-10-01", ""),
    ("last March", "2026-03-01..2026-04-01", ""),
    ("last november", "2025-11-01..2025-12-01", ""),
    ("this november", "2026-11-01..2026-12-01", ""),
    // relative
    ("today", "2026-10-30..2026-10-31", ""),
    ("yesterday", "2026-10-29..2026-10-30", ""),
    ("this week", "2026-10-26..2026-11-02", ""),
    ("last week", "2026-10-19..2026-10-26", ""),
    ("this month", "2026-10-01..2026-11-01", ""),
    ("last month", "2026-09-01..2026-10-01", ""),
    ("this year", "2026-01-01..2027-01-01", ""),
    ("last year", "2025-01-01..2026-01-01", ""),
    ("last weekend", "2026-10-24..2026-10-26", ""),
    ("this weekend", "2026-10-31..2026-11-02", ""),
    // seasons
    ("summer", "Summer", ""),
    ("last summer", "Summer 2026", ""),
    ("this winter", "Winter 2025", ""),
    ("last winter", "Winter 2025", ""),
    ("last autumn", "Autumn 2025", ""),
    ("this fall", "Autumn 2026", ""),
    ("summer of 2015", "Summer 2015", ""),
    ("winter 2019 snow", "Winter 2019", "snow"),
    // weekdays and holidays
    ("weekends", "days 1100000", ""),
    ("weekdays", "days 0011111", ""),
    ("sundays at the park", "days 1000000", "park"),
    ("christmas", "Christmas", ""),
    ("Xmas 2019", "Christmas 2019", ""),
    ("last christmas", "Christmas 2025", ""),
    ("new year's eve", "NewYearsEve", ""),
    ("new year 2020", "NewYear 2020", ""),
    ("Easter 2024", "Easter 2024", ""),
    ("diwali", "Diwali", ""),
    ("last diwali", "Diwali 2025", ""),
    ("halloween costumes", "Halloween", "costumes"),
    ("valentine's day", "Valentines", ""),
    ("thanksgiving 2024", "Thanksgiving 2024", ""),
    // kinds
    ("videos", "Video", ""),
    ("screenshots", "Screenshot", ""),
    (
        "screen shots from 2020",
        "Screenshot | 2020-01-01..2021-01-01",
        "",
    ),
    ("selfies", "Selfie", ""),
    ("raw", "Raw", ""),
    ("live photos", "Live", ""),
    ("favourites", "favourite", ""),
    (
        "my favorites from 2019",
        "favourite | 2019-01-01..2020-01-01",
        "",
    ),
    ("starred videos", "favourite | Video", ""),
    // names
    ("Lisbon", "Album:Lisbon", ""),
    ("lisbon 2019", "Album:Lisbon | 2019-01-01..2020-01-01", ""),
    ("Paris", "Place:Paris", ""),
    ("Paris Hilton", "Person:Paris Hilton", ""),
    ("paris hilton hotel", "Person:Paris Hilton", "hotel"),
    ("sao paulo", "Place:São Paulo", ""),
    ("SÃO PAULO at night", "Place:São Paulo", "night"),
    ("new york", "Place:New York", ""),
    ("new york city skyline", "Place:New York City", "skyline"),
    ("new year in new york", "NewYear | Place:New York", ""),
    ("Ana in Kerala", "Person:Ana | Place:Kerala", ""),
    ("ana and june", "Person:Ana | Person:June", ""),
    ("june 2019", "2019-06-01..2019-07-01", ""),
    ("summer 2019", "Album:Summer 2019", ""),
    ("canon eos r5", "Camera:Canon EOS R5", ""),
    ("shot on iphone 15 pro", "Camera:iPhone 15 Pro", "shot"),
    ("rf 24-70mm f2.8", "Lens:RF 24-70mm F2.8", ""),
    ("wedding videos", "Album:Wedding | Video", ""),
    // free text
    ("dog on a beach in Lisbon", "Album:Lisbon", "dog on a beach"),
    ("dog", "", "dog"),
    ("birthday cake", "", "birthday cake"),
    (
        "red car 2019 portugal",
        "2019-01-01..2020-01-01 | Place:Portugal",
        "red car",
    ),
    ("the", "", ""),
    ("", "", ""),
    ("   ", "", ""),
    ("receipt from may", "month 5", "receipt"),
];

#[test]
fn table() {
    let v = vocab();
    let cx = cx();
    let mut failures = Vec::new();
    for (q, want_f, want_text) in CASES {
        let got = parse(q, &v, &cx);
        let f: Vec<String> = got.filters.iter().map(|f| show(&f.what)).collect();
        let f = f.join(" | ");
        if f != *want_f || got.free_text != *want_text {
            failures.push(format!(
                "{q:?}\n   want: [{want_f}] {want_text:?}\n    got: [{f}] {:?}",
                got.free_text
            ));
        }
    }
    assert!(CASES.len() >= 60);
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn spans_and_labels_cover_the_input() {
    let q = "dogs in SÃO PAULO, march 2021";
    let got = parse(q, &vocab(), &cx());
    let spans: Vec<&str> = got.filters.iter().map(|f| &q[f.span.clone()]).collect();
    assert_eq!(spans, ["SÃO PAULO", "march 2021"]);
    let labels: Vec<&str> = got.filters.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, ["São Paulo", "march 2021"]);
    assert_eq!(got.free_text, "dogs");
}

#[test]
fn seasons_follow_the_photo_hemisphere() {
    let f = DateFilter::Season {
        season: shoebox_search::query::Season::Summer,
        year: Some(2019),
    };
    let at = |y, m, d| Zone::UTC.start_of(Date::new(y, m, d));
    assert!(f.matches(at(2019, 7, 10), Zone::UTC, false));
    assert!(!f.matches(at(2019, 7, 10), Zone::UTC, true));
    // Southern summer 2019 runs from December 2019 to February 2020.
    assert!(f.matches(at(2020, 1, 10), Zone::UTC, true));
    assert!(!f.matches(at(2019, 1, 10), Zone::UTC, true));
}

#[test]
fn holidays_match_their_days() {
    let at = |y, m, d| Zone::UTC.start_of(Date::new(y, m, d));
    let h = |holiday, year| DateFilter::Holiday { holiday, year };
    use shoebox_search::query::Holiday::*;
    assert!(h(NewYear, Some(2020)).matches(at(2019, 12, 31), Zone::UTC, false));
    assert!(h(NewYear, Some(2020)).matches(at(2020, 1, 1), Zone::UTC, false));
    assert!(!h(NewYear, Some(2020)).matches(at(2020, 12, 31), Zone::UTC, false));
    assert!(h(Easter, None).matches(at(2019, 4, 19), Zone::UTC, false));
    assert!(!h(Easter, None).matches(at(2019, 4, 18), Zone::UTC, false));
    assert!(h(Diwali, None).matches(at(2023, 11, 12), Zone::UTC, false));
    assert!(h(Christmas, None).matches(at(1999, 12, 25), Zone::UTC, false));
}

#[test]
fn parsing_is_fast_with_a_large_vocabulary() {
    let mut v = vocab();
    for i in 0..10_000 {
        v.add(NameKind::Place, &format!("Place number {i}"));
    }
    let cx = cx();
    let q = "Ana in place number 9999 last summer dog on a beach christmas 2019";
    let start = Instant::now();
    let n = 200;
    for _ in 0..n {
        std::hint::black_box(parse(q, &v, &cx));
    }
    let per = start.elapsed() / n;
    // Under 1 ms (S1-02); debug builds of our own crates are unoptimised, so allow headroom there.
    let budget = if cfg!(debug_assertions) { 5 } else { 1 };
    assert!(per.as_millis() < budget, "{per:?} per query");
}

#[test]
fn suggestions_prefer_whole_name_prefixes() {
    let v = vocab();
    let s: Vec<&str> = v
        .suggest("new", 5)
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(s, ["New York", "New York City"]);
    let s: Vec<&str> = v.suggest("pa", 5).iter().map(|e| e.name.as_str()).collect();
    assert_eq!(s, ["Paris", "Paris Hilton", "São Paulo"]);
}
