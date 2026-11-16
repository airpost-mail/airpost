// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The date in a reply's attribution line and a forward's header (`formatQuoteDate`,
//! `date-format.ts`), filled from the pattern the desktop takes from Intl
//! (`composing.quoteTexts.dateTime`).

use chrono::{Datelike, TimeZone, Timelike};

use crate::settings::QuoteTexts;

/// Fills `texts.date_time` for `at` (epoch seconds) in `tz`. Fields: `{month}` (a name from
/// `months`), `{monthNum}`, `{month2}`, `{day}`, `{day2}`, `{year}`, `{year2}`, `{hour}`,
/// `{hour2}`, `{minute}` and `{dayPeriod}` (from `dayPeriods`). With a `{dayPeriod}` the hour
/// is on a 12-hour clock.
pub fn format_quote_date(at: i64, texts: &QuoteTexts, tz: chrono_tz::Tz) -> String {
    let Some(t) = tz.timestamp_opt(at, 0).single() else {
        return String::new();
    };
    let twelve = texts.date_time.contains("{dayPeriod}");
    let hour = if twelve {
        match t.hour() % 12 {
            0 => 12,
            h => h,
        }
    } else {
        t.hour()
    };
    let month0 = t.month0() as usize;
    let month = texts
        .months
        .get(month0)
        .cloned()
        .unwrap_or_else(|| t.month().to_string());
    let period = texts
        .day_periods
        .get(usize::from(t.hour() >= 12))
        .cloned()
        .unwrap_or_else(|| if t.hour() >= 12 { "PM" } else { "AM" }.into());
    let fields: [(&str, String); 11] = [
        ("{monthNum}", t.month().to_string()),
        ("{month2}", format!("{:02}", t.month())),
        ("{month}", month),
        ("{day2}", format!("{:02}", t.day())),
        ("{day}", t.day().to_string()),
        ("{year2}", format!("{:02}", t.year().rem_euclid(100))),
        ("{year}", t.year().to_string()),
        ("{hour2}", format!("{hour:02}")),
        ("{hour}", hour.to_string()),
        ("{minute}", format!("{:02}", t.minute())),
        ("{dayPeriod}", period),
    ];
    // One pass, so a month name holding a field's braces stays as it is.
    let pattern = &texts.date_time;
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut rest = pattern.as_str();
    'outer: while !rest.is_empty() {
        if rest.starts_with('{') {
            for (field, value) in &fields {
                if let Some(after) = rest.strip_prefix(field) {
                    out.push_str(value);
                    rest = after;
                    continue 'outer;
                }
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_default() {
        let t = QuoteTexts::default();
        assert_eq!(
            format_quote_date(1700000000, &t, chrono_tz::America::Chicago),
            "Nov 14, 2023, 4:13 PM"
        );
        assert_eq!(
            format_quote_date(1700000000, &t, chrono_tz::UTC),
            "Nov 14, 2023, 10:13 PM"
        );
        assert_eq!(
            format_quote_date(1699920000, &t, chrono_tz::UTC),
            "Nov 14, 2023, 12:00 AM"
        );
    }
}
