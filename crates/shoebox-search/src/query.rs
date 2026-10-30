//! Turning what someone typed into filters and leftover words (epic §6 step 1).
//!
//! The parser knows dates, kinds of media and the names the library contains (its
//! [`Vocabulary`]). Everything it does not recognise is left as free text for ranking. Each
//! filter keeps its span in the input so the UI can show it as a removable chip.

use crate::calendar::{diwali, easter, nth_weekday, Date, Zone};
use crate::lang::{Lang, Word, ENGLISH};
use crate::text::{fold, tokenize, words, Token};
use shoebox_core::ids::Timestamp;
use shoebox_core::store::TimeRange;
use std::collections::HashMap;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    /// First month of the season in the northern hemisphere; seasons are three months long.
    fn first_month(self, southern: bool) -> u8 {
        let north = match self {
            Season::Spring => 3,
            Season::Summer => 6,
            Season::Autumn => 9,
            Season::Winter => 12,
        };
        if southern {
            (north + 5) % 12 + 1
        } else {
            north
        }
    }

    /// The season's year containing `d` (the year it starts in), if `d` is in this season.
    fn year_of(self, d: Date, southern: bool) -> Option<i32> {
        let first = self.first_month(southern);
        let offset = (i32::from(d.month) - i32::from(first)).rem_euclid(12);
        if offset >= 3 {
            return None;
        }
        Some(if d.month < first { d.year - 1 } else { d.year })
    }

    fn start(self, year: i32, southern: bool) -> Date {
        Date::new(year, self.first_month(southern), 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Holiday {
    /// 24–26 December.
    Christmas,
    /// 31 December and 1 January; its year is January's.
    NewYear,
    NewYearsEve,
    /// Good Friday to Easter Monday.
    Easter,
    /// The day, a day either side.
    Diwali,
    Halloween,
    Valentines,
    /// Fourth Thursday of November (US) and the weekend after it.
    Thanksgiving,
}

impl Holiday {
    /// The holiday's dates in `year` (inclusive).
    fn dates(self, year: i32) -> Option<(Date, Date)> {
        let d = Date::new;
        Some(match self {
            Holiday::Christmas => (d(year, 12, 24), d(year, 12, 26)),
            Holiday::NewYear => (d(year - 1, 12, 31), d(year, 1, 1)),
            Holiday::NewYearsEve => (d(year, 12, 31), d(year, 12, 31)),
            Holiday::Easter => {
                let e = easter(year);
                (e.add_days(-2), e.add_days(1))
            }
            Holiday::Diwali => {
                let w = diwali(year)?;
                (w.add_days(-1), w.add_days(1))
            }
            Holiday::Halloween => (d(year, 10, 31), d(year, 10, 31)),
            Holiday::Valentines => (d(year, 2, 14), d(year, 2, 14)),
            Holiday::Thanksgiving => {
                let t = nth_weekday(year, 11, 3, 4);
                (t, t.add_days(3))
            }
        })
    }

    fn contains(self, date: Date, year: Option<i32>) -> bool {
        // New Year spans two calendar years; check the next one as well.
        [date.year, date.year + 1]
            .into_iter()
            .filter(|y| year.is_none_or(|want| want == *y))
            .filter_map(|y| self.dates(y))
            .any(|(a, b)| a <= date && date <= b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relative {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    LastYear,
}

/// When a photo was taken. Ranges are absolute; the rest are checked against the local date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DateFilter {
    /// `[start, end)` in UTC (computed from local dates in the query's zone).
    Range(TimeRange),
    /// A month in any year (1..=12).
    Month(u8),
    /// A season, in the photo's hemisphere (from its GPS, else the device's).
    Season {
        season: Season,
        year: Option<i32>,
    },
    /// Bit `n` set = weekday `n` (0 = Monday).
    Weekdays(u8),
    Holiday {
        holiday: Holiday,
        year: Option<i32>,
    },
}

impl DateFilter {
    /// Does a photo taken at `t` match? `southern` is the photo's hemisphere.
    pub fn matches(&self, t: Timestamp, zone: Zone, southern: bool) -> bool {
        if let DateFilter::Range(r) = self {
            return r.contains(t);
        }
        let d = zone.date(t);
        match self {
            DateFilter::Range(_) => unreachable!(),
            DateFilter::Month(m) => d.month == *m,
            DateFilter::Season { season, year } => season
                .year_of(d, southern)
                .is_some_and(|y| year.is_none_or(|want| want == y)),
            DateFilter::Weekdays(mask) => mask & (1 << d.weekday()) != 0,
            DateFilter::Holiday { holiday, year } => holiday.contains(d, *year),
        }
    }

    /// The absolute range this filter is limited to, if any (to narrow a scan by time).
    pub fn range(&self) -> Option<TimeRange> {
        match self {
            DateFilter::Range(r) => Some(*r),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MediaKind {
    Photo,
    Video,
    Screenshot,
    Selfie,
    Raw,
    Live,
}

/// What kind of thing a vocabulary name is. The order is the preference when one name is
/// several kinds at once (a person called like an album).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameKind {
    Person,
    Album,
    Place,
    Camera,
    Lens,
    /// A thing the content index knows ("dog", "beach"). Suggested, but left in the free text so
    /// it is ranked rather than required.
    Label,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Date(DateFilter),
    Media(MediaKind),
    Favourite,
    Name { kind: NameKind, name: String },
}

/// One recognised piece of the query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub what: FilterKind,
    /// Byte span in the input.
    pub span: Range<usize>,
    /// What the chip shows: the name as the library spells it, or the input's own words.
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub filters: Vec<Filter>,
    /// The words no filter took, connectives at their edges dropped.
    pub free_text: String,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty() && self.free_text.is_empty()
    }
}

/// A name the library contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: NameKind,
    pub name: String,
    words: Vec<String>,
}

/// What the library contains: camera, place, album and people's names, and label words.
#[derive(Clone, Debug, Default)]
pub struct Vocabulary {
    entries: Vec<Entry>,
    /// First folded word → entries starting with it.
    by_first: HashMap<String, Vec<u32>>,
    seen: HashMap<(NameKind, Vec<String>), u32>,
}

impl Vocabulary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a name (a repeat of the same kind and words is ignored).
    pub fn add(&mut self, kind: NameKind, name: &str) {
        let w = words(name);
        if w.is_empty() || self.seen.contains_key(&(kind, w.clone())) {
            return;
        }
        let idx = self.entries.len() as u32;
        self.seen.insert((kind, w.clone()), idx);
        self.by_first.entry(w[0].clone()).or_default().push(idx);
        self.entries.push(Entry {
            kind,
            name: name.trim().to_string(),
            words: w,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The longest entry matching `words[0..]`: (length in words, entry).
    fn longest(&self, words: &[&str]) -> Option<(usize, &Entry)> {
        let first = words.first()?;
        self.by_first
            .get(*first)?
            .iter()
            .map(|&i| &self.entries[i as usize])
            .filter(|e| {
                e.words.len() <= words.len() && e.words.iter().zip(words).all(|(a, b)| a == b)
            })
            .max_by(|a, b| a.words.len().cmp(&b.words.len()).then(b.kind.cmp(&a.kind)))
            .map(|e| (e.words.len(), e))
    }

    /// Names some word of which starts with `prefix` (folded), best first: whole-name prefix
    /// matches, then shorter names.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<&Entry> {
        let p = fold(prefix.trim());
        if p.is_empty() {
            return vec![];
        }
        let pw = words(&p);
        let mut hits: Vec<(bool, usize, &Entry)> = self
            .entries
            .iter()
            .filter_map(|e| {
                let whole = fold(&e.name).starts_with(&p);
                let word = pw.len() == 1 && e.words.iter().any(|w| w.starts_with(&pw[0]));
                (whole || word).then_some((!whole, e.name.len(), e))
            })
            .collect();
        hits.sort_by(|a, b| (a.0, a.1, &a.2.name).cmp(&(b.0, b.1, &b.2.name)));
        hits.into_iter().take(limit).map(|h| h.2).collect()
    }
}

/// When the query is parsed: what "now" and "this year" mean.
#[derive(Clone, Copy, Debug)]
pub struct ParseContext {
    pub now: Timestamp,
    pub zone: Zone,
    /// The device is in the southern hemisphere (for "last summer").
    pub southern: bool,
}

/// Parse with the English word list.
pub fn parse(input: &str, vocab: &Vocabulary, cx: &ParseContext) -> Query {
    parse_with(&ENGLISH, input, vocab, cx)
}

pub fn parse_with(lang: &Lang, input: &str, vocab: &Vocabulary, cx: &ParseContext) -> Query {
    let tokens = tokenize(input);
    let words: Vec<&str> = tokens.iter().map(|t| t.word.as_str()).collect();
    let mut filters = Vec::new();
    let mut taken = vec![false; tokens.len()];
    let mut i = 0;
    while i < tokens.len() {
        let p = Parser {
            lang,
            input,
            tokens: &tokens,
            words: &words,
            cx,
        };
        let name = vocab
            .longest(&words[i..])
            .filter(|(_, e)| e.kind != NameKind::Label);
        let other = p.date(i).or_else(|| p.single(i));
        let pick = match (name, other) {
            // Longest wins; on a tie the library's own name wins.
            (Some((n, e)), Some((m, _))) if n >= m => Some((n, name_filter(e))),
            (Some((n, e)), None) => Some((n, name_filter(e))),
            (_, Some((m, f))) => Some((m, f)),
            (None, None) => None,
        };
        match pick {
            Some((n, what)) => {
                let span = tokens[i].start..tokens[i + n - 1].end;
                let label = match &what {
                    FilterKind::Name { name, .. } => name.clone(),
                    _ => input[span.clone()].to_string(),
                };
                filters.push(Filter { what, span, label });
                taken[i..i + n].iter_mut().for_each(|t| *t = true);
                i += n;
            }
            None => i += 1,
        }
    }
    Query {
        filters,
        free_text: leftover(lang, input, &tokens, &taken),
    }
}

fn name_filter(e: &Entry) -> FilterKind {
    FilterKind::Name {
        kind: e.kind,
        name: e.name.clone(),
    }
}

/// Untaken runs of tokens, each trimmed of connectives at its edges, joined by a space.
fn leftover(lang: &Lang, input: &str, tokens: &[Token], taken: &[bool]) -> String {
    let mut runs: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if taken[i] {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < tokens.len() && !taken[j] {
            j += 1;
        }
        let is_conn = |t: &Token| lang.connectives.contains(&t.word.as_str());
        let run = &tokens[i..j];
        let a = run.iter().position(|t| !is_conn(t));
        let b = run.iter().rposition(|t| !is_conn(t));
        if let (Some(a), Some(b)) = (a, b) {
            runs.push(&input[run[a].start..run[b].end]);
        }
        i = j;
    }
    runs.join(" ")
}

struct Parser<'a> {
    lang: &'a Lang,
    input: &'a str,
    tokens: &'a [Token],
    words: &'a [&'a str],
    cx: &'a ParseContext,
}

fn year_of(w: &str) -> Option<i32> {
    if w.len() == 4 && w.bytes().all(|b| b.is_ascii_digit()) {
        let y: i32 = w.parse().ok()?;
        (1800..=2200).contains(&y).then_some(y)
    } else {
        None
    }
}

/// "90s", "1990s", "2010s".
fn decade_of(w: &str) -> Option<i32> {
    let d = w.strip_suffix('s')?;
    if !d.bytes().all(|b| b.is_ascii_digit()) || !d.ends_with('0') {
        return None;
    }
    match d.len() {
        4 => d.parse().ok(),
        2 => d
            .parse::<i32>()
            .ok()
            .map(|y| if y >= 30 { 1900 + y } else { 2000 + y }),
        _ => None,
    }
}

impl Parser<'_> {
    fn word(&self, i: usize) -> Option<(usize, Word)> {
        self.lang.longest(self.words.get(i..)?)
    }

    fn year(&self, i: usize) -> Option<i32> {
        self.words.get(i).and_then(|w| year_of(w))
    }

    /// A year after `i`, optionally after "of"/"in": (words used, year).
    fn year_after(&self, i: usize) -> Option<(usize, i32)> {
        if let Some(y) = self.year(i) {
            return Some((1, y));
        }
        let w = self.words.get(i)?;
        if self.lang.of.contains(w) {
            return self.year(i + 1).map(|y| (2, y));
        }
        None
    }

    fn range(&self, from: Date, to_exclusive: Date) -> DateFilter {
        DateFilter::Range(TimeRange {
            start: Some(self.cx.zone.start_of(from)),
            end: Some(self.cx.zone.start_of(to_exclusive)),
        })
    }

    fn year_range(&self, a: i32, b: i32) -> DateFilter {
        self.range(Date::new(a.min(b), 1, 1), Date::new(a.max(b) + 1, 1, 1))
    }

    fn month_range(&self, year: i32, month: u8) -> DateFilter {
        let next = if month == 12 {
            Date::new(year + 1, 1, 1)
        } else {
            Date::new(year, month + 1, 1)
        };
        self.range(Date::new(year, month, 1), next)
    }

    fn today(&self) -> Date {
        self.cx.zone.date(self.cx.now)
    }

    /// Is the text between tokens `i` and `i + 1` a dash ("2018-2020", "2018 – 2020")?
    fn dash_between(&self, i: usize) -> bool {
        let (Some(a), Some(b)) = (self.tokens.get(i), self.tokens.get(i + 1)) else {
            return false;
        };
        let gap = self.input[a.end..b.start].trim();
        matches!(gap, "-" | "–" | "—" | "..")
    }

    /// A date phrase starting at `i`: (words used, filter).
    fn date(&self, i: usize) -> Option<(usize, FilterKind)> {
        let (n, f) = self.date_inner(i)?;
        Some((n, FilterKind::Date(f)))
    }

    fn date_inner(&self, i: usize) -> Option<(usize, DateFilter)> {
        let w = self.words.get(i)?;
        if let Some(a) = year_of(w) {
            // "2018-2020", "2018 to 2020"
            if self.dash_between(i) {
                if let Some(b) = self.year(i + 1) {
                    return Some((2, self.year_range(a, b)));
                }
            }
            if matches!(self.word(i + 1), Some((1, Word::To))) {
                if let Some(b) = self.year(i + 2) {
                    return Some((3, self.year_range(a, b)));
                }
            }
            // "2019 summer", "2021 March" are rare; a bare year is the common case.
            return Some((1, self.year_range(a, a)));
        }
        if let Some(d) = decade_of(w) {
            return Some((1, self.year_range(d, d + 9)));
        }
        let (n, word) = self.word(i)?;
        match word {
            Word::Month(m) => match self.year_after(i + n) {
                Some((k, y)) => Some((n + k, self.month_range(y, m))),
                None => Some((n, DateFilter::Month(m))),
            },
            Word::Season(season) => {
                let year = self.year_after(i + n);
                Some((
                    n + year.map_or(0, |(k, _)| k),
                    DateFilter::Season {
                        season,
                        year: year.map(|(_, y)| y),
                    },
                ))
            }
            Word::Holiday(holiday) => {
                let year = self.year_after(i + n);
                Some((
                    n + year.map_or(0, |(k, _)| k),
                    DateFilter::Holiday {
                        holiday,
                        year: year.map(|(_, y)| y),
                    },
                ))
            }
            Word::Weekday(d) => Some((n, DateFilter::Weekdays(1 << d))),
            Word::Weekend => Some((n, DateFilter::Weekdays(0b110_0000))),
            Word::Weekdays => Some((n, DateFilter::Weekdays(0b001_1111))),
            Word::Relative(r) => Some((n, self.relative(r))),
            Word::Last | Word::This => {
                let last = word == Word::Last;
                let (k, next) = self.word(i + n)?;
                let f = match next {
                    Word::Season(s) => self.season_occurrence(s, last),
                    Word::Holiday(h) => self.holiday_occurrence(h, last)?,
                    Word::Month(m) => self.month_occurrence(m, last),
                    Word::Weekend => self.weekend_occurrence(last),
                    _ => return None,
                };
                Some((n + k, f))
            }
            _ => None,
        }
    }

    fn relative(&self, r: Relative) -> DateFilter {
        let t = self.today();
        let monday = t.add_days(-i64::from(t.weekday()));
        let month_start = Date::new(t.year, t.month, 1);
        let prev_month = month_start.add_days(-1);
        let (a, b) = match r {
            Relative::Today => (t, t.add_days(1)),
            Relative::Yesterday => (t.add_days(-1), t),
            Relative::ThisWeek => (monday, monday.add_days(7)),
            Relative::LastWeek => (monday.add_days(-7), monday),
            Relative::ThisMonth => (month_start, month_start.add_days(31).with_day(1)),
            Relative::LastMonth => (Date::new(prev_month.year, prev_month.month, 1), month_start),
            Relative::ThisYear => (Date::new(t.year, 1, 1), Date::new(t.year + 1, 1, 1)),
            Relative::LastYear => (Date::new(t.year - 1, 1, 1), Date::new(t.year, 1, 1)),
        };
        self.range(a, b)
    }

    /// "last summer": the latest summer that has ended; "this summer": the latest that has
    /// started. In the device's hemisphere; each photo is then checked in its own.
    fn season_occurrence(&self, s: Season, last: bool) -> DateFilter {
        let t = self.today();
        let south = self.cx.southern;
        let year = (t.year - 2..=t.year)
            .rev()
            .find(|&y| {
                let start = s.start(y, south);
                let end = start.add_days(92).with_day(1);
                if last {
                    end <= t
                } else {
                    start <= t
                }
            })
            .unwrap_or(t.year - 1);
        DateFilter::Season {
            season: s,
            year: Some(year),
        }
    }

    fn holiday_occurrence(&self, h: Holiday, last: bool) -> Option<DateFilter> {
        let t = self.today();
        let year = (t.year - 2..=t.year + 1).rev().find(|&y| {
            h.dates(y)
                .is_some_and(|(a, b)| if last { b < t } else { a <= t })
        })?;
        Some(DateFilter::Holiday {
            holiday: h,
            year: Some(year),
        })
    }

    /// "last March": the latest March that has ended; "this March": this year's.
    fn month_occurrence(&self, m: u8, last: bool) -> DateFilter {
        let t = self.today();
        let year = if !last || t.month > m {
            t.year
        } else {
            t.year - 1
        };
        self.month_range(year, m)
    }

    fn weekend_occurrence(&self, last: bool) -> DateFilter {
        let t = self.today();
        // Weeks start on Monday: "this weekend" is this week's, "last weekend" the one before.
        let saturday = t.add_days(5 - i64::from(t.weekday()));
        let sat = if last {
            saturday.add_days(-7)
        } else {
            saturday
        };
        self.range(sat, sat.add_days(2))
    }

    /// A one-word kind or favourite.
    fn single(&self, i: usize) -> Option<(usize, FilterKind)> {
        let (n, w) = self.word(i)?;
        match w {
            Word::Media(k) => Some((n, FilterKind::Media(k))),
            Word::Favourite => Some((n, FilterKind::Favourite)),
            _ => None,
        }
    }
}

trait WithDay {
    fn with_day(self, day: u8) -> Self;
}

impl WithDay for Date {
    fn with_day(self, day: u8) -> Self {
        Date::new(self.year, self.month, day)
    }
}
