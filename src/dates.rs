//! Upstream date helpers: `unified_timestamp` with the pinned `DATE_FORMATS`,
//! `extract_timezone`, `email.utils.parsedate_tz` and `strftime_or_none`.
//! Python `strptime` is reproduced for the directives these formats use in
//! the C locale. Non-ASCII decimal digits are not interpreted, and inputs for
//! which upstream raises (years outside 1..=9999) return `None`.

use crate::batch::is_py_space;
use regex::Regex;
use std::sync::LazyLock;

const DATE_FORMATS: &[&str] = &[
    "%d %B %Y",
    "%d %b %Y",
    "%B %d %Y",
    "%B %dst %Y",
    "%B %dnd %Y",
    "%B %drd %Y",
    "%B %dth %Y",
    "%b %d %Y",
    "%b %dst %Y",
    "%b %dnd %Y",
    "%b %drd %Y",
    "%b %dth %Y",
    "%b %dst %Y %I:%M",
    "%b %dnd %Y %I:%M",
    "%b %drd %Y %I:%M",
    "%b %dth %Y %I:%M",
    "%Y %m %d",
    "%Y-%m-%d",
    "%Y.%m.%d.",
    "%Y/%m/%d",
    "%Y/%m/%d %H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y%m%d%H%M",
    "%Y%m%d%H%M%S",
    "%Y%m%d",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d %H:%M:%S.%f",
    "%Y-%m-%d %H:%M:%S:%f",
    "%d.%m.%Y %H:%M",
    "%d.%m.%Y %H.%M",
    "%Y-%m-%dT%H:%M:%SZ",
    "%Y-%m-%dT%H:%M:%S.%fZ",
    "%Y-%m-%dT%H:%M:%S.%f0Z",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M:%S.%f",
    "%Y-%m-%dT%H:%M",
    "%b %d %Y at %H:%M",
    "%b %d %Y at %H:%M:%S",
    "%B %d %Y at %H:%M",
    "%B %d %Y at %H:%M:%S",
    "%H:%M %d-%b-%Y",
];

const DAY_FIRST: &[&str] = &[
    "%d-%m-%Y",
    "%d.%m.%Y",
    "%d.%m.%y",
    "%d/%m/%Y",
    "%d/%m/%y",
    "%d/%m/%Y %H:%M:%S",
    "%d-%m-%Y %H:%M",
    "%H:%M %d/%m/%Y",
];

const MONTH_FIRST: &[&str] = &[
    "%m-%d-%Y",
    "%m.%d.%Y",
    "%m/%d/%Y",
    "%m/%d/%y",
    "%m/%d/%Y %H:%M:%S",
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const TIMEZONE_NAMES: &[(&str, i64)] = &[
    ("UT", 0),
    ("UTC", 0),
    ("GMT", 0),
    ("Z", 0),
    ("AST", -4),
    ("ADT", -3),
    ("EST", -5),
    ("EDT", -4),
    ("CST", -6),
    ("CDT", -5),
    ("MST", -7),
    ("MDT", -6),
    ("PST", -8),
    ("PDT", -7),
];

/// A naive `datetime` without fractions (they never reach `timegm`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct DateTime {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `calendar.timegm`: `datetime.date(year, month, 1)` must be valid; the
/// remaining fields are plain arithmetic.
fn timegm(value: &DateTime) -> Option<i64> {
    if !(1..=9999).contains(&value.year) || !(1..=12).contains(&value.month) {
        return None;
    }
    let days = days_from_civil(value.year, value.month, 1).checked_add(value.day - 1)?;
    days.checked_mul(24)?
        .checked_add(value.hour)?
        .checked_mul(60)?
        .checked_add(value.minute)?
        .checked_mul(60)?
        .checked_add(value.second)
}

/// Naive `datetime + timedelta(seconds=…)`, which overflows outside 1..=9999.
fn shift(value: &DateTime, seconds: i64) -> Option<DateTime> {
    let total = timegm(value)?.checked_add(seconds)?;
    let (year, month, day) = civil_from_days(total.div_euclid(86_400));
    let rest = total.rem_euclid(86_400);
    (1..=9999).contains(&year).then_some(DateTime {
        year,
        month,
        day,
        hour: rest / 3600,
        minute: rest % 3600 / 60,
        second: rest % 60,
    })
}

/// Python `re` `\s+` for `str` patterns.
const SPACE: &str = r"[\s\x1c-\x1f]+";

/// The `_strptime` regex for a format, in the C locale.
fn strptime_regex(format: &str) -> Regex {
    let mut pattern = String::from("^(?i:");
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let directive = chars.next().expect("pinned formats are well formed");
            pattern.push_str(match directive {
                'd' => "(?P<d>3[0-1]|[1-2][0-9]|0[1-9]|[1-9]| [1-9])",
                'f' => "(?P<f>[0-9]{1,6})",
                'H' => "(?P<H>2[0-3]|[0-1][0-9]|[0-9]| [0-9])",
                'I' => "(?P<I>1[0-2]|0[1-9]|[1-9]| [1-9])",
                'm' => "(?P<m>1[0-2]|0[1-9]|[1-9])",
                'M' => "(?P<M>[0-5][0-9]|[0-9])",
                'S' => "(?P<S>6[0-1]|[0-5][0-9]|[0-9])",
                'y' => "(?P<y>[0-9][0-9])",
                'Y' => "(?P<Y>[0-9][0-9][0-9][0-9])",
                // Longest names first, stable among equal lengths.
                'B' => {
                    "(?P<B>september|february|november|december|january|october|august|march|april|june|july|may)"
                }
                'b' => "(?P<b>jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)",
                other => unreachable!("unsupported directive %{other}"),
            });
        } else if is_py_space(c) {
            while chars.peek().is_some_and(|next| is_py_space(*next)) {
                chars.next();
            }
            pattern.push_str(SPACE);
        } else if c == '\'' {
            pattern.push_str("['\u{2bc}]");
        } else {
            pattern.push_str(&regex::escape(&c.to_string()));
        }
    }
    pattern.push(')');
    Regex::new(&pattern).expect("strptime patterns are valid")
}

static DAY_FIRST_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DATE_FORMATS
        .iter()
        .chain(DAY_FIRST)
        .map(|format| strptime_regex(format))
        .collect()
});

static MONTH_FIRST_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DATE_FORMATS
        .iter()
        .chain(MONTH_FIRST)
        .map(|format| strptime_regex(format))
        .collect()
});

/// `datetime.strptime(data, format)` for a compiled pinned format.
fn strptime(regex: &Regex, data: &str) -> Option<DateTime> {
    let found = regex.captures(data)?;
    if found.get(0)?.end() != data.len() {
        return None;
    }
    let number = |name: &str| {
        found
            .name(name)
            .map(|m| m.as_str().trim_start().parse::<i64>())
    };
    let month_name = |name: &str, length: usize| {
        found.name(name).map(|m| {
            let lower = m.as_str().to_lowercase();
            MONTHS
                .iter()
                .position(|month| month[..length.min(month.len())] == lower)
                .map(|index| index as i64 + 1)
        })
    };
    let mut value = DateTime {
        year: 1900,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    if let Some(year) = number("y") {
        let year = year.ok()?;
        value.year = if year <= 68 { year + 2000 } else { year + 1900 };
    }
    if let Some(year) = number("Y") {
        value.year = year.ok()?;
    }
    if let Some(month) = number("m") {
        value.month = month.ok()?;
    }
    if let Some(month) = month_name("B", usize::MAX) {
        value.month = month?;
    }
    if let Some(month) = month_name("b", 3) {
        value.month = month?;
    }
    if let Some(day) = number("d") {
        value.day = day.ok()?;
    }
    if let Some(hour) = number("H") {
        value.hour = hour.ok()?;
    }
    // Without %p, 12 o'clock is midnight.
    if let Some(hour) = number("I") {
        let hour = hour.ok()?;
        value.hour = if hour == 12 { 0 } else { hour };
    }
    if let Some(minute) = number("M") {
        value.minute = minute.ok()?;
    }
    if let Some(second) = number("S") {
        value.second = second.ok()?;
    }
    // `datetime(...)` validation.
    let valid = (1..=9999).contains(&value.year)
        && (1..=days_in_month(value.year, value.month)).contains(&value.day)
        && (0..60).contains(&value.second);
    valid.then_some(value)
}

/// Python `int(text)` for ASCII digits with an optional sign and underscores.
fn py_int(text: &str) -> Option<i64> {
    let text = text.trim_matches(is_py_space);
    let (negative, digits) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let value: i64 = digits.replace('_', "").parse().ok()?;
    Some(if negative { -value } else { value })
}

/// `email.utils.parsedate_tz` without the (unused) timezone offset.
fn parsedate(data: &str) -> Option<DateTime> {
    const MONTH_NAMES: [&str; 24] = [
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let mut data: Vec<String> = data
        .split(is_py_space)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if data.is_empty() {
        return None;
    }
    if data[0].ends_with(',') || DAY_NAMES.contains(&data[0].to_lowercase().as_str()) {
        data.remove(0);
    } else if let Some(index) = data[0].rfind(',') {
        data[0] = data[0][index + 1..].to_owned();
    }
    if data.len() == 3 {
        let stuff: Vec<String> = data[0].split('-').map(str::to_owned).collect();
        if stuff.len() == 3 {
            data = stuff.into_iter().chain(data[1..].iter().cloned()).collect();
        }
    }
    if data.len() == 4 {
        let s = data[3].clone();
        let index = s.find('+').or_else(|| s.find('-'));
        match index {
            Some(index) if index > 0 => {
                data.truncate(3);
                data.push(s[..index].to_owned());
                data.push(s[index..].to_owned());
            }
            _ => data.push(String::new()),
        }
    }
    if data.len() < 5 {
        return None;
    }
    let (mut dd, mut mm, mut yy, mut tm) = (
        data[0].clone(),
        data[1].clone(),
        data[2].clone(),
        data[3].clone(),
    );
    let mut tz = data[4].clone();
    if dd.is_empty() || mm.is_empty() || yy.is_empty() {
        return None;
    }
    mm = mm.to_lowercase();
    if !MONTH_NAMES.contains(&mm.as_str()) {
        (dd, mm) = (mm, dd.to_lowercase());
        if !MONTH_NAMES.contains(&mm.as_str()) {
            return None;
        }
    }
    let mut month = MONTH_NAMES.iter().position(|name| *name == mm)? as i64 + 1;
    if month > 12 {
        month -= 12;
    }
    if dd.ends_with(',') {
        dd.pop();
    }
    if yy.find(':').is_some_and(|index| index > 0) {
        std::mem::swap(&mut yy, &mut tm);
    }
    if yy.ends_with(',') {
        yy.pop();
        if yy.is_empty() {
            return None;
        }
    }
    if !yy.chars().next().is_some_and(char::is_numeric) {
        std::mem::swap(&mut yy, &mut tz);
    }
    if tm.ends_with(',') {
        tm.pop();
    }
    let parts: Vec<&str> = tm.split(':').collect();
    let (hour, minute, second) = match parts.as_slice() {
        [hour, minute] => (*hour, *minute, "0"),
        [hour, minute, second] => (*hour, *minute, *second),
        [single] if single.contains('.') => match single.split('.').collect::<Vec<_>>()[..] {
            [hour, minute] => (hour, minute, "0"),
            [hour, minute, second] => (hour, minute, second),
            _ => return None,
        },
        _ => return None,
    };
    let mut year = py_int(&yy)?;
    let day = py_int(&dd)?;
    let (hour, minute, second) = (py_int(hour)?, py_int(minute)?, py_int(second)?);
    if year < 100 {
        year += if year > 68 { 1900 } else { 2000 };
    }
    Some(DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_decimal(c: char) -> bool {
    static DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d$").unwrap());
    let mut buffer = [0; 4];
    DIGIT.is_match(c.encode_utf8(&mut buffer))
}

/// `extract_timezone(date_str, default=None)`: the offset in seconds (zero
/// when absent) and the remaining text.
fn extract_timezone(date: &str) -> (i64, String) {
    let chars: Vec<char> = date.chars().collect();
    let boundary = |index: usize| {
        let before = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        before.is_some_and(is_word) != chars.get(index).copied().is_some_and(is_word)
    };
    let all = |range: std::ops::Range<usize>, test: fn(char) -> bool| {
        chars[range].iter().copied().all(test)
    };
    // `(?<=.\b\d{4}|\b\d{2}:\d\d)` at `p`.
    let preceded = |p: usize| {
        p >= 5
            && ((chars[p - 5] != '\n' && boundary(p - 4) && all(p - 4..p, is_decimal))
                || (boundary(p - 5)
                    && all(p - 5..p - 3, is_decimal)
                    && chars[p - 3] == ':'
                    && all(p - 2..p, is_decimal)))
    };
    // `(?<!.\b[a-zA-Z]{3}|[a-zA-Z]{4}|..\b\d\d)` at `p`.
    let excluded = |p: usize| {
        (p >= 4
            && chars[p - 4] != '\n'
            && boundary(p - 3)
            && all(p - 3..p, |c| c.is_ascii_alphabetic()))
            || (p >= 4 && all(p - 4..p, |c| c.is_ascii_alphabetic()))
            || (p >= 4
                && chars[p - 4] != '\n'
                && chars[p - 3] != '\n'
                && boundary(p - 2)
                && all(p - 2..p, is_decimal))
    };
    let offset = |p: usize| -> Option<i64> {
        let mut index = p;
        if chars.get(index) == Some(&' ') {
            index += 1;
        }
        let sign = match chars.get(index)? {
            '+' => 1,
            '-' => -1,
            _ => return None,
        };
        index += 1;
        let two = |index: usize| -> Option<i64> {
            let digits = chars.get(index..index + 2)?;
            digits
                .iter()
                .all(char::is_ascii_digit)
                .then(|| digits.iter().collect::<String>().parse().ok())?
        };
        let hours = two(index)?;
        index += 2;
        if chars.get(index) == Some(&':') {
            index += 1;
        }
        let minutes = two(index)?;
        (index + 2 == chars.len()).then_some(sign * (hours * 3600 + minutes * 60))
    };
    for p in 8..chars.len() {
        if chars[p..] == ['Z'] {
            return (0, chars[..p].iter().collect());
        }
        if (preceded(p) || !excluded(p))
            && let Some(seconds) = offset(p)
        {
            return (seconds, chars[..p].iter().collect());
        }
    }
    static NAMED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\d{1,2}:\d{1,2}(?:\.\d+)?(?P<tz>[\s\x1c-\x1f]*[A-Z]+)$").unwrap()
    });
    if let Some(tz) = NAMED.captures(date).and_then(|found| found.name("tz"))
        && let Some((_, hours)) = TIMEZONE_NAMES
            .iter()
            .find(|(name, _)| *name == tz.as_str().trim_matches(is_py_space))
    {
        return (hours * 3600, date[..tz.start()].to_owned());
    }
    (0, date.to_owned())
}

/// Upstream `unified_timestamp(date_str, day_first)` (no `tz_offset`).
pub fn unified_timestamp(date: &str, day_first: bool) -> Option<i64> {
    static NAMES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)[,|]|(mon|tues?|wed(nes)?|thu(rs)?|fri|sat(ur)?|sun)(day)?").unwrap()
    });
    static MERIDIEM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)[\s\x1c-\x1f]*(?:AM|PM)(?:[\s\x1c-\x1f]+[A-Z]+)?").unwrap()
    });
    static PM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)PM").unwrap());
    static UNKNOWN_ZONE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\d{1,2}:\d{1,2}(?:\.\d+)?(?P<tz>[\s\x1c-\x1f]*[A-Z]+)$").unwrap()
    });
    static NANOSECONDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^([0-9]{4,}-[0-9]{1,2}-[0-9]{1,2}T[0-9]{1,2}:[0-9]{1,2}:[0-9]{1,2}\.[0-9]{6})[0-9]+$",
        )
        .unwrap()
    });
    let stripped = NAMES.replace_all(date, "");
    let mut collapsed = String::with_capacity(stripped.len());
    let mut in_space = false;
    for c in stripped.chars() {
        if is_py_space(c) {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    let pm_delta = if PM.is_match(&collapsed) { 12 } else { 0 };
    let (timezone, rest) = extract_timezone(&collapsed);
    let mut rest = MERIDIEM.replace_all(&rest, "").into_owned();
    if let Some(tz) = UNKNOWN_ZONE
        .captures(&rest)
        .and_then(|found| found.name("tz"))
    {
        rest.truncate(tz.start());
    }
    if let Some(found) = NANOSECONDS.captures(&rest) {
        rest = found[1].to_owned();
    }
    let regexes = if day_first {
        &*DAY_FIRST_REGEXES
    } else {
        &*MONTH_FIRST_REGEXES
    };
    for regex in regexes {
        if let Some(parsed) = strptime(regex, &rest) {
            // A shifted datetime outside 1..=9999 overflows upstream.
            let shifted = shift(&shift(&parsed, -timezone)?, pm_delta * 3600)?;
            return timegm(&shifted);
        }
    }
    let parsed = parsedate(&rest)?;
    timegm(&parsed)?
        .checked_add(pm_delta * 3600)?
        .checked_sub(timezone)
}

/// Upstream `strftime_or_none(timestamp)` with the default `%Y%m%d`.
pub fn strftime_date(timestamp: i64) -> Option<String> {
    let (year, month, day) = civil_from_days(timestamp.div_euclid(86_400));
    (1..=9999)
        .contains(&year)
        .then(|| format!("{year:04}{month:02}{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_unified_timestamp_and_strftime_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/dates/cases.json")).unwrap();
        for case in fixture["unified_timestamp"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            for (key, day_first) in [("day_first", true), ("month_first", false)] {
                assert_eq!(
                    unified_timestamp(input, day_first),
                    case[key].as_i64(),
                    "{input:?} {key}"
                );
            }
        }
        for case in fixture["strftime"].as_array().unwrap() {
            let timestamp = case["timestamp"].as_i64().unwrap();
            assert_eq!(
                strftime_date(timestamp).as_deref(),
                case["date"].as_str(),
                "{timestamp}"
            );
        }
    }
}
