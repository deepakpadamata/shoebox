//! The words the parser knows, per language. English only for now; another language is another
//! [`Lang`] value with the same shape.

use crate::query::{Holiday, MediaKind, Relative, Season};

/// A recognised word or phrase (before it is combined with its neighbours into a filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    Month(u8),
    /// 0 = Monday.
    Weekday(u8),
    Weekend,
    Weekdays,
    Season(Season),
    Holiday(Holiday),
    Relative(Relative),
    Media(MediaKind),
    Favourite,
    /// "last", "previous": the latest past occurrence of what follows.
    Last,
    /// "this": the occurrence of what follows that contains (or most recently started before) now.
    This,
    /// Joins two years into a range ("2018 to 2020").
    To,
}

pub struct Lang {
    /// Phrases, each a sequence of folded words.
    pub phrases: &'static [(&'static [&'static str], Word)],
    /// Words dropped from the ends of leftover text ("dog on a beach in" → "dog on a beach").
    pub connectives: &'static [&'static str],
    /// Words that may sit between a month or season and its year ("summer of 2019").
    pub of: &'static [&'static str],
}

impl Lang {
    /// The longest phrase starting at `words[0]`: its length in words and what it means.
    pub fn longest(&self, words: &[&str]) -> Option<(usize, Word)> {
        self.phrases
            .iter()
            .filter(|(p, _)| p.len() <= words.len() && p.iter().zip(words).all(|(a, b)| a == b))
            .max_by_key(|(p, _)| p.len())
            .map(|(p, w)| (p.len(), *w))
    }
}

use Word::*;

pub static ENGLISH: Lang = Lang {
    phrases: &[
        (&["january"], Month(1)),
        (&["jan"], Month(1)),
        (&["february"], Month(2)),
        (&["feb"], Month(2)),
        (&["march"], Month(3)),
        (&["mar"], Month(3)),
        (&["april"], Month(4)),
        (&["apr"], Month(4)),
        (&["may"], Month(5)),
        (&["june"], Month(6)),
        (&["jun"], Month(6)),
        (&["july"], Month(7)),
        (&["jul"], Month(7)),
        (&["august"], Month(8)),
        (&["aug"], Month(8)),
        (&["september"], Month(9)),
        (&["sept"], Month(9)),
        (&["sep"], Month(9)),
        (&["october"], Month(10)),
        (&["oct"], Month(10)),
        (&["november"], Month(11)),
        (&["nov"], Month(11)),
        (&["december"], Month(12)),
        (&["dec"], Month(12)),
        (&["monday"], Weekday(0)),
        (&["mondays"], Weekday(0)),
        (&["tuesday"], Weekday(1)),
        (&["tuesdays"], Weekday(1)),
        (&["wednesday"], Weekday(2)),
        (&["wednesdays"], Weekday(2)),
        (&["thursday"], Weekday(3)),
        (&["thursdays"], Weekday(3)),
        (&["friday"], Weekday(4)),
        (&["fridays"], Weekday(4)),
        (&["saturday"], Weekday(5)),
        (&["saturdays"], Weekday(5)),
        (&["sunday"], Weekday(6)),
        (&["sundays"], Weekday(6)),
        (&["weekend"], Weekend),
        (&["weekends"], Weekend),
        (&["weekday"], Weekdays),
        (&["weekdays"], Weekdays),
        (&["spring"], Season(Season::Spring)),
        (&["summer"], Season(Season::Summer)),
        (&["autumn"], Season(Season::Autumn)),
        (&["fall"], Season(Season::Autumn)),
        (&["winter"], Season(Season::Winter)),
        (&["christmas"], Holiday(Holiday::Christmas)),
        (&["xmas"], Holiday(Holiday::Christmas)),
        (&["new", "year"], Holiday(Holiday::NewYear)),
        (&["new", "years"], Holiday(Holiday::NewYear)),
        (&["new", "years", "eve"], Holiday(Holiday::NewYearsEve)),
        (&["new", "year", "eve"], Holiday(Holiday::NewYearsEve)),
        (&["nye"], Holiday(Holiday::NewYearsEve)),
        (&["easter"], Holiday(Holiday::Easter)),
        (&["diwali"], Holiday(Holiday::Diwali)),
        (&["deepavali"], Holiday(Holiday::Diwali)),
        (&["halloween"], Holiday(Holiday::Halloween)),
        (&["valentines"], Holiday(Holiday::Valentines)),
        (&["valentines", "day"], Holiday(Holiday::Valentines)),
        (&["valentine"], Holiday(Holiday::Valentines)),
        (&["valentine", "day"], Holiday(Holiday::Valentines)),
        (&["thanksgiving"], Holiday(Holiday::Thanksgiving)),
        (&["today"], Relative(Relative::Today)),
        (&["yesterday"], Relative(Relative::Yesterday)),
        (&["this", "week"], Relative(Relative::ThisWeek)),
        (&["last", "week"], Relative(Relative::LastWeek)),
        (&["this", "month"], Relative(Relative::ThisMonth)),
        (&["last", "month"], Relative(Relative::LastMonth)),
        (&["this", "year"], Relative(Relative::ThisYear)),
        (&["last", "year"], Relative(Relative::LastYear)),
        (&["photos"], Media(MediaKind::Photo)),
        (&["photo"], Media(MediaKind::Photo)),
        (&["pictures"], Media(MediaKind::Photo)),
        (&["pics"], Media(MediaKind::Photo)),
        (&["videos"], Media(MediaKind::Video)),
        (&["video"], Media(MediaKind::Video)),
        (&["movies"], Media(MediaKind::Video)),
        (&["clips"], Media(MediaKind::Video)),
        (&["screenshots"], Media(MediaKind::Screenshot)),
        (&["screenshot"], Media(MediaKind::Screenshot)),
        (&["screen", "shots"], Media(MediaKind::Screenshot)),
        (&["selfies"], Media(MediaKind::Selfie)),
        (&["selfie"], Media(MediaKind::Selfie)),
        (&["raw"], Media(MediaKind::Raw)),
        (&["raws"], Media(MediaKind::Raw)),
        (&["live", "photos"], Media(MediaKind::Live)),
        (&["favourites"], Favourite),
        (&["favorites"], Favourite),
        (&["favourite"], Favourite),
        (&["favorite"], Favourite),
        (&["faves"], Favourite),
        (&["starred"], Favourite),
        (&["last"], Last),
        (&["previous"], Last),
        (&["this"], This),
        (&["to"], To),
        (&["until"], To),
        (&["till"], To),
        (&["through"], To),
    ],
    connectives: &[
        "a", "an", "the", "in", "on", "at", "from", "of", "during", "taken", "with", "and", "near",
        "around", "for", "by", "my", "our",
    ],
    of: &["of", "in"],
};
