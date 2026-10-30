//! Civil dates without a date library: enough to turn "March 2021", "weekends" or "Easter" into
//! ranges and per-photo checks. Proleptic Gregorian calendar; days count from 1970-01-01.

use shoebox_core::ids::Timestamp;

pub const MS_PER_DAY: i64 = 86_400_000;

/// A calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
}

impl Date {
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
    pub fn days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(z: i64) -> Self {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(self) -> u8 {
        // 1970-01-01 was a Thursday (3).
        (self.days() + 3).rem_euclid(7) as u8
    }

    pub fn add_days(self, n: i64) -> Self {
        Self::from_days(self.days() + n)
    }

    pub fn days_in_month(year: i32, month: u8) -> u8 {
        match month {
            2 if is_leap(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Local time zone as a fixed offset from UTC, in minutes (UTC+5:30 = 330).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Zone {
    pub offset_min: i32,
}

impl Zone {
    pub const UTC: Zone = Zone { offset_min: 0 };

    /// The local date of a moment.
    pub fn date(self, t: Timestamp) -> Date {
        let local = t.0 + i64::from(self.offset_min) * 60_000;
        Date::from_days(local.div_euclid(MS_PER_DAY))
    }

    /// The moment local midnight starts `d`.
    pub fn start_of(self, d: Date) -> Timestamp {
        Timestamp(d.days() * MS_PER_DAY - i64::from(self.offset_min) * 60_000)
    }
}

/// Easter Sunday (Western), anonymous Gregorian algorithm.
pub fn easter(year: i32) -> Date {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    Date::new(year, month as u8, day as u8)
}

/// The `n`th (1-based) `weekday` (0 = Monday) of a month.
pub fn nth_weekday(year: i32, month: u8, weekday: u8, n: u8) -> Date {
    let first = Date::new(year, month, 1);
    let shift = (i64::from(weekday) - i64::from(first.weekday())).rem_euclid(7);
    first.add_days(shift + 7 * (i64::from(n) - 1))
}

/// Diwali (Lakshmi Puja) by year. It follows the lunar calendar, so it is a table; dates are as
/// observed in India and may differ by a day elsewhere (matching allows a day either side).
const DIWALI: &[(i32, u8, u8)] = &[
    (2000, 10, 26),
    (2001, 11, 14),
    (2002, 11, 4),
    (2003, 10, 25),
    (2004, 11, 12),
    (2005, 11, 1),
    (2006, 10, 21),
    (2007, 11, 9),
    (2008, 10, 28),
    (2009, 10, 17),
    (2010, 11, 5),
    (2011, 10, 26),
    (2012, 11, 13),
    (2013, 11, 3),
    (2014, 10, 23),
    (2015, 11, 11),
    (2016, 10, 30),
    (2017, 10, 19),
    (2018, 11, 7),
    (2019, 10, 27),
    (2020, 11, 14),
    (2021, 11, 4),
    (2022, 10, 24),
    (2023, 11, 12),
    (2024, 11, 1),
    (2025, 10, 21),
    (2026, 11, 8),
    (2027, 10, 29),
    (2028, 10, 17),
    (2029, 11, 5),
    (2030, 10, 26),
];

pub fn diwali(year: i32) -> Option<Date> {
    DIWALI
        .iter()
        .find(|(y, _, _)| *y == year)
        .map(|&(y, m, d)| Date::new(y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_trip() {
        for z in [-800_000, -1, 0, 1, 59, 60, 10_957, 19_000, 2_000_000] {
            assert_eq!(Date::from_days(z).days(), z);
        }
        assert_eq!(Date::new(1970, 1, 1).days(), 0);
        assert_eq!(Date::new(2000, 3, 1).days(), 11_017);
        assert_eq!(Date::from_days(-1), Date::new(1969, 12, 31));
    }

    #[test]
    fn weekdays() {
        assert_eq!(Date::new(1970, 1, 1).weekday(), 3);
        assert_eq!(Date::new(2026, 10, 30).weekday(), 4); // a Friday
        assert_eq!(Date::new(1999, 12, 31).weekday(), 4);
    }

    #[test]
    fn easter_dates() {
        assert_eq!(easter(2019), Date::new(2019, 4, 21));
        assert_eq!(easter(2024), Date::new(2024, 3, 31));
        assert_eq!(easter(2025), Date::new(2025, 4, 20));
        assert_eq!(easter(2000), Date::new(2000, 4, 23));
    }

    #[test]
    fn thanksgiving() {
        assert_eq!(nth_weekday(2024, 11, 3, 4), Date::new(2024, 11, 28));
        assert_eq!(nth_weekday(2026, 11, 3, 4), Date::new(2026, 11, 26));
    }

    #[test]
    fn zones_shift_the_date() {
        let t = Zone::UTC.start_of(Date::new(2020, 1, 1));
        assert_eq!(Zone { offset_min: -60 }.date(t), Date::new(2019, 12, 31));
        assert_eq!(Zone { offset_min: 330 }.date(t), Date::new(2020, 1, 1));
        let ist = Zone { offset_min: 330 };
        assert_eq!(
            ist.date(ist.start_of(Date::new(2020, 1, 1))),
            Date::new(2020, 1, 1)
        );
    }
}
