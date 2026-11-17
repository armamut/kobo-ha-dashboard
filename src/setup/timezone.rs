//! The zone of the computer the setup tool runs on, as a POSIX `TZ` rule.
//!
//! A Kobo keeps its clock in UTC and Nickel never exports its own zone, so
//! anything started from NickelMenu draws UTC. The launcher can hand the
//! dashboard a zone through `TZ`, but the device has no timezone database: an
//! IANA name such as `Europe/Amsterdam` cannot be resolved there. A POSIX rule
//! (`<+01>-1<+02>-2,M3.5.0/2:00,M10.5.0/3:00`) needs no database and follows
//! daylight saving, unlike the fixed `theme.utc_offset`, which is an hour off
//! after every clock change.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike, Utc};

/// How far apart the clock changes of the year are sampled.
const STEP_HOURS: i64 = 6;

/// The rule for this computer, or `None` when its zone cannot be expressed (a
/// year with three offsets, for instance).
pub fn posix_rule() -> Option<Rule> {
    let year = Utc::now().year();
    // A zone that changes shape in this year only - daylight saving abolished in
    // the middle of it, say - has no usable shape now, while next year does, and
    // next year is what has to keep working.
    for start in [year, year + 1] {
        let Some(text) = rule_for(start) else { continue };
        // A POSIX rule repeats every year. Where the real zone does not (Israel
        // starts on the Friday before the last Sunday of March, which moves), the
        // rule is right for now and drifts later; the install log says so.
        let repeats = rule_for(start + 1).as_deref() == Some(text.as_str());
        return Some(Rule { text, repeats });
    }
    None
}

/// A POSIX `TZ` rule for this computer.
pub struct Rule {
    /// The rule itself, e.g. `<+01>-1<+02>-2,M3.5.0/2:00,M10.5.0/3:00`.
    pub text: String,
    /// Whether the same rule also describes next year, so the clock keeps being
    /// right without a new install.
    pub repeats: bool,
}

fn rule_for(year: i32) -> Option<String> {
    let start = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single()?;
    let changes = clock_changes(start, year);
    if changes.is_empty() {
        let offset = offset_at(start);
        return Some(format!("{}{}", zone_name(offset), posix_offset(offset)));
    }

    let mut offsets: Vec<i32> = changes.iter().flat_map(|(_, from, to)| [*from, *to]).collect();
    offsets.sort_unstable();
    offsets.dedup();
    // Two offsets and two changes: one into daylight saving and one back. A year
    // with more (a zone that changed its standard offset) is left to the user.
    if offsets.len() != 2 || changes.len() != 2 {
        return None;
    }
    let (std, dst) = (offsets[0], offsets[1]);
    // Daylight saving moves the clock forward, so the larger offset is the
    // summer one - in the southern hemisphere too, where it starts in October.
    let into_dst = changes.iter().find(|(_, from, to)| *from == std && *to == dst)?;
    let into_std = changes.iter().find(|(_, from, to)| *from == dst && *to == std)?;

    Some(format!(
        "{}{}{}{},{},{}",
        zone_name(std),
        posix_offset(std),
        zone_name(dst),
        posix_offset(dst),
        transition_rule(into_dst.0, std)?,
        transition_rule(into_std.0, dst)?,
    ))
}

/// The offsets the clock steps through in the given year, as
/// (first instant with the new offset, offset before, offset after).
fn clock_changes(start: DateTime<Utc>, year: i32) -> Vec<(DateTime<Utc>, i32, i32)> {
    let mut changes = Vec::new();
    let mut at = start;
    let mut offset = offset_at(at);
    while at.year() == year {
        let next = at + Duration::hours(STEP_HOURS);
        let after = offset_at(next);
        if after != offset {
            changes.push((first_instant_with(next, at, offset), offset, after));
            offset = after;
        }
        at = next;
    }
    changes
}

/// Narrow the gap between `before` (still the old offset) and `after` (the new
/// one) down to a minute: a rule names a time, not a second.
fn first_instant_with(after: DateTime<Utc>, before: DateTime<Utc>, offset: i32) -> DateTime<Utc> {
    let (mut lo, mut hi) = (before, after);
    while hi - lo > Duration::minutes(1) {
        let mid = lo + (hi - lo) / 2;
        if offset_at(mid) == offset {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

fn offset_at(at: DateTime<Utc>) -> i32 {
    at.with_timezone(&Local).offset().local_minus_utc()
}

/// `M3.5.0/2:00`: the month, the week within it (5 = the last), the weekday
/// (0 = Sunday) and the local time of the change, counted before it happens.
fn transition_rule(at: DateTime<Utc>, offset_before: i32) -> Option<String> {
    // The instant shifted into the zone as it was just before the change.
    let local = at + Duration::seconds(offset_before as i64);
    let day = local.day();
    let week = if day + 7 > days_in_month(local.year(), local.month())? {
        5
    } else {
        (day - 1) / 7 + 1
    };
    Some(format!(
        "M{}.{}.{}/{}:{:02}",
        local.month(),
        week,
        local.weekday().num_days_from_sunday(),
        local.hour(),
        local.minute()
    ))
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let (year, month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    Some(first.pred_opt()?.day())
}

/// The name of a zone: a POSIX name has to be letters, so the offset itself is
/// written in angle brackets. Nothing shows it; only the offset matters.
fn zone_name(offset: i32) -> String {
    let (sign, abs) = if offset < 0 { ('-', -offset) } else { ('+', offset) };
    let (hours, minutes) = (abs / 3600, (abs % 3600) / 60);
    if minutes == 0 {
        format!("<{}{:02}>", sign, hours)
    } else {
        format!("<{}{:02}:{:02}>", sign, hours, minutes)
    }
}

/// POSIX counts the hours *west* of Greenwich, so Central Europe (one hour
/// ahead) is written as `-1`.
fn posix_offset(offset: i32) -> String {
    let posix = -offset;
    if posix == 0 {
        return "0".to_string();
    }
    let (sign, abs) = if posix < 0 { ('-', -posix) } else { ('+', posix) };
    let (hours, minutes) = (abs / 3600, (abs % 3600) / 60);
    if minutes == 0 {
        format!("{}{}", sign, hours)
    } else {
        format!("{}{}:{:02}", sign, hours, minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn central_europe_keeps_its_two_rules() {
        let std = 3600;
        let dst = 7200;
        // 2027: the last Sunday of March is the 28th, of October the 31st.
        let rule = format!(
            "{}{}{}{},{},{}",
            zone_name(std),
            posix_offset(std),
            zone_name(dst),
            posix_offset(dst),
            transition_rule(at("2027-03-28T01:00:00Z"), std).unwrap(),
            transition_rule(at("2027-10-31T01:00:00Z"), dst).unwrap(),
        );
        assert_eq!(rule, "<+01>-1<+02>-2,M3.5.0/2:00,M10.5.0/3:00");
    }

    #[test]
    fn a_zone_without_daylight_saving_is_one_offset() {
        assert_eq!(format!("{}{}", zone_name(0), posix_offset(0)), "<+00>0");
        assert_eq!(format!("{}{}", zone_name(-18000), posix_offset(-18000)), "<-05>+5");
    }

    #[test]
    fn the_week_of_a_change_is_counted_or_last() {
        // 2027-03-14 is the second Sunday of March (United States).
        assert_eq!(transition_rule(at("2027-03-14T07:00:00Z"), -18000).unwrap(), "M3.2.0/2:00");
        // 2027-11-07 is the first Sunday of November; the clock goes back at
        // 2:00 daylight time, which is the local time the rule names.
        assert_eq!(transition_rule(at("2027-11-07T06:00:00Z"), -14400).unwrap(), "M11.1.0/2:00");
        // 2027-10-03 is the first Sunday of October (southern hemisphere).
        assert_eq!(transition_rule(at("2027-10-03T02:00:00Z"), 36000).unwrap(), "M10.1.0/12:00");
    }

    #[test]
    fn a_change_at_the_end_of_a_month_is_the_last_one() {
        // 2027-03-28 is the fourth Sunday and the last of March.
        assert!(transition_rule(at("2027-03-28T01:00:00Z"), 3600).unwrap().starts_with("M3.5.0/"));
        // 2027-03-21 is the third and not the last.
        assert!(transition_rule(at("2027-03-21T01:00:00Z"), 3600).unwrap().starts_with("M3.3.0/"));
    }

    #[test]
    fn a_half_hour_offset_keeps_its_minutes() {
        assert_eq!(posix_offset(19800), "-5:30");
        assert_eq!(zone_name(19800), "<+05:30>");
        assert_eq!(posix_offset(-12600), "+3:30");
    }

    #[test]
    fn this_computer_yields_a_rule_that_reads_as_posix() {
        // Whatever the machine's zone is, the rule must be usable: a name, an
        // offset, and - with daylight saving - two transition rules.
        let rule = posix_rule().expect("a zone for this computer");
        let text = rule.text;
        assert!(text.starts_with('<'), "{text}");
        let parts: Vec<&str> = text.split(',').collect();
        assert!(parts.len() == 1 || parts.len() == 3, "{text}");
        for part in parts.iter().skip(1) {
            assert!(part.starts_with('M'), "{text}");
            assert!(part.contains('/'), "{text}");
        }
        // Central European rules are the same every year; a zone whose dates
        // move would report false here, and the installer warns about it.
        assert!(rule.repeats, "this machine's rule does not repeat: {text}");
    }
}

