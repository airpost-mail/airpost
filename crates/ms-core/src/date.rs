//! Lenient Date header parsing for message ids (RFC 5322 section 3.3 and its obsolete
//! forms in 4.3), plus IMAP INTERNALDATE.
//!
//! Accepted, beyond the strict grammar: a missing day name or seconds, 2- and 3-digit years,
//! named and military zones, a missing zone (taken as UTC), trailing comments such as
//! `(PST)`, and asctime order (`Mon Feb  7 21:52:25 1994`). Anything without a day, month,
//! year and hour:minute is rejected.

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Epoch seconds of a Date header value, or `None` when it cannot be read.
pub fn parse_rfc5322(value: &str) -> Option<i64> {
    let cleaned = strip_comments(value).replace(',', " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();

    let mut month: Option<u32> = None;
    let mut time: Option<(u32, u32, u32)> = None;
    let mut offset: Option<i64> = None;
    let mut numbers: Vec<&str> = Vec::new();

    for tok in tokens {
        if time.is_none() && tok.contains(':') {
            let (t, zone) = split_time_and_zone(tok);
            time = Some(parse_time(t)?);
            if let Some(z) = zone {
                offset = parse_zone(z);
            }
            continue;
        }
        if tok.bytes().all(|b| b.is_ascii_digit()) {
            numbers.push(tok);
            continue;
        }
        if month.is_none() && tok.len() >= 3 {
            let prefix = tok[..3].to_ascii_lowercase();
            if let Some(m) = MONTHS.iter().position(|m| *m == prefix) {
                month = Some(m as u32 + 1);
                continue;
            }
        }
        if offset.is_none() && time.is_some() {
            if let Some(o) = parse_zone(tok) {
                offset = Some(o);
            }
        }
    }

    let month = month?;
    let (hour, minute, second) = time?;
    let (day, year) = day_and_year(&numbers)?;
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let second = second.min(59);
    Some(
        days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64
            - offset.unwrap_or(0),
    )
}

/// IMAP INTERNALDATE (`17-Jul-1996 02:44:25 -0700`, RFC 3501 section 9) as epoch seconds.
pub fn parse_internaldate(value: &str) -> Option<i64> {
    let value = value.trim().trim_matches('"');
    let mut parts = value.split_whitespace();
    let date = parts.next()?;
    let time = parts.next()?;
    let zone = parts.next().unwrap_or("+0000");
    let mut d = date.split('-');
    let day: u32 = d.next()?.trim().parse().ok()?;
    let month_name = d.next()?.to_ascii_lowercase();
    let month = MONTHS.iter().position(|m| month_name.starts_with(m))? as u32 + 1;
    let year: i64 = d.next()?.parse().ok()?;
    let (hour, minute, second) = parse_time(time)?;
    let offset = parse_zone(zone)?;
    Some(
        days_from_civil(year, month, day) * 86_400
            + hour as i64 * 3600
            + minute as i64 * 60
            + second.min(59) as i64
            - offset,
    )
}

fn strip_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut depth = 0u32;
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// `21:52:25-0800` happens; split off a zone glued to the time.
fn split_time_and_zone(tok: &str) -> (&str, Option<&str>) {
    match tok.find(['+', '-']) {
        Some(p) => (&tok[..p], Some(&tok[p..])),
        None => (tok, None),
    }
}

fn parse_time(t: &str) -> Option<(u32, u32, u32)> {
    let mut it = t.split(':');
    let h: u32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let s: u32 = match it.next() {
        Some(s) => s.split('.').next()?.parse().ok()?,
        None => 0,
    };
    Some((h, m, s))
}

fn parse_zone(z: &str) -> Option<i64> {
    let bytes = z.as_bytes();
    if (bytes.first() == Some(&b'+') || bytes.first() == Some(&b'-'))
        && bytes.len() >= 3
        && bytes[1..].iter().all(|b| b.is_ascii_digit())
    {
        let sign = if bytes[0] == b'-' { -1 } else { 1 };
        let digits = &z[1..];
        let (h, m) = match digits.len() {
            4 => (
                digits[..2].parse::<i64>().ok()?,
                digits[2..].parse::<i64>().ok()?,
            ),
            2 => (digits.parse::<i64>().ok()?, 0),
            3 => (
                digits[..1].parse::<i64>().ok()?,
                digits[1..].parse::<i64>().ok()?,
            ),
            _ => return None,
        };
        if h > 23 || m > 59 {
            return None;
        }
        return Some(sign * (h * 3600 + m * 60));
    }
    let hours = match z.to_ascii_uppercase().as_str() {
        "UT" | "UTC" | "GMT" | "Z" => 0,
        "EST" => -5,
        "EDT" => -4,
        "CST" => -6,
        "CDT" => -5,
        "MST" => -7,
        "MDT" => -6,
        "PST" => -8,
        "PDT" => -7,
        // RFC 5322 4.3: military zones were specified backwards, so treat them as -0000.
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => 0,
        _ => return None,
    };
    Some(hours * 3600)
}

fn day_and_year(numbers: &[&str]) -> Option<(u32, i64)> {
    let year_idx = numbers
        .iter()
        .position(|n| n.len() >= 3)
        .or(if numbers.len() >= 2 {
            Some(numbers.len() - 1)
        } else {
            None
        })?;
    let day_idx = (0..numbers.len()).find(|i| *i != year_idx)?;
    let day: u32 = numbers[day_idx].parse().ok()?;
    let raw_year: i64 = numbers[year_idx].parse().ok()?;
    let year = match numbers[year_idx].len() {
        1 | 2 if raw_year < 50 => 2000 + raw_year,
        1 | 2 => 1900 + raw_year,
        3 => 1900 + raw_year,
        _ => raw_year,
    };
    Some((day, year))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc5322_forms() {
        assert_eq!(parse_rfc5322("Thu, 01 Jan 1970 00:00:00 +0000"), Some(0));
        assert_eq!(
            parse_rfc5322("Mon, 7 Feb 1994 21:52:25 -0800"),
            Some(760_686_745)
        );
        assert_eq!(
            parse_rfc5322("7 Feb 1994 21:52:25 -0800 (PST)"),
            Some(760_686_745)
        );
        assert_eq!(
            parse_rfc5322("Mon, 7 Feb 1994 21:52:25 PST"),
            Some(760_686_745)
        );
        assert_eq!(
            parse_rfc5322("Mon, 7 Feb 94 21:52:25 -0800"),
            Some(760_686_745)
        );
        assert_eq!(
            parse_rfc5322("Mon Feb  7 21:52:25 1994"),
            Some(760_686_745 - 8 * 3600)
        );
        assert_eq!(
            parse_rfc5322("Tue, 1 Jul 2003 10:52 +0200"),
            Some(1_057_049_520)
        );
        assert_eq!(
            parse_rfc5322("Sat, 29 Feb 2020 12:00:00 GMT"),
            Some(1_582_977_600)
        );
        assert_eq!(
            parse_rfc5322("Sat, 29 Feb 2020 12:00:00"),
            Some(1_582_977_600)
        );
        assert_eq!(
            parse_rfc5322("Sat, 29 Feb 2020 12:00:00 +0000 (UTC)"),
            Some(1_582_977_600)
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_rfc5322(""), None);
        assert_eq!(parse_rfc5322("yesterday"), None);
        assert_eq!(parse_rfc5322("Mon, 7 Feb 1994"), None);
        assert_eq!(parse_rfc5322("Mon, 40 Feb 1994 10:00:00"), None);
    }

    #[test]
    fn internaldate() {
        assert_eq!(
            parse_internaldate("17-Jul-1996 02:44:25 -0700"),
            Some(837_596_665)
        );
        assert_eq!(
            parse_internaldate("\" 1-Jan-1970 00:00:00 +0000\""),
            Some(0)
        );
    }
}
