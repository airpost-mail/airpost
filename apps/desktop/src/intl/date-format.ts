// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
One rule for showing times (docs/tech-debt.md TD48): Intl.DateTimeFormat in the app's locale, with
a 24-hour clock when `core.workspace.use24HourClock` is on and the locale's own clock when it is
off (so a German user who never touched the setting keeps 24 hours, as moment's `LT` gave them).
Use these instead of moment's `LT` or a hard-coded `h:mm A`.

The reply attribution line and the forward header carry a date the engine writes too, when it
builds a draft (`draft.create`, docs/desktop-client-plan.md 3.3). The engine has no Intl, so the
settings mirror gives it `composing.quoteTexts` (quoteTexts below): the localized templates, and
the date pattern taken from Intl's own parts in the user's locale and hour cycle, which it fills in
the same way formatQuoteDate does here.
*/
import { localizerFor } from './localize';

export type HourCycle = 'h12' | 'h23';

export const USE_24_HOUR_KEY = 'core.workspace.use24HourClock';

function configGet(key: string): unknown {
  const env = typeof AppEnv !== 'undefined' ? AppEnv : null;
  return env && env.config ? env.config.get(key) : undefined;
}

export function appLocale(): string {
  return (typeof navigator !== 'undefined' && navigator.language) || 'en-US';
}

/** The clock `locale` uses by default. */
export function localeHourCycle(locale: string = appLocale()): HourCycle {
  try {
    const hc = new Intl.DateTimeFormat(locale, { hour: 'numeric' }).resolvedOptions().hourCycle;
    return hc === 'h11' || hc === 'h12' ? 'h12' : 'h23';
  } catch (err) {
    return 'h12';
  }
}

export function hourCycle(use24 = configGet(USE_24_HOUR_KEY), locale = appLocale()): HourCycle {
  return use24 ? 'h23' : localeHourCycle(locale);
}

/** A time of day: "2:05 PM" or "14:05". */
export function formatTime(
  date: Date,
  {
    seconds,
    timeZone,
    timeZoneName,
    locale = appLocale(),
    cycle = hourCycle(configGet(USE_24_HOUR_KEY), locale),
  }: {
    seconds?: boolean;
    timeZone?: string;
    timeZoneName?: boolean;
    cycle?: HourCycle;
    locale?: string;
  } = {}
) {
  return new Intl.DateTimeFormat(locale, {
    hour: 'numeric',
    minute: '2-digit',
    second: seconds ? '2-digit' : undefined,
    hourCycle: cycle,
    timeZone,
    timeZoneName: timeZoneName ? 'short' : undefined,
  }).format(date);
}

/**
 * A time as compact as the clock allows: a 12-hour clock drops ":00" on the hour ("10 AM"); a
 * 24-hour clock keeps it ("10:00"), where a bare "10" would not read as a time.
 */
export function formatShortTime(
  date: Date,
  {
    timeZone,
    locale = appLocale(),
    cycle = hourCycle(configGet(USE_24_HOUR_KEY), locale),
  }: { timeZone?: string; cycle?: HourCycle; locale?: string } = {}
) {
  const minutes = Number(
    new Intl.DateTimeFormat('en-US', { minute: 'numeric', timeZone }).format(date)
  );
  if (cycle === 'h12' && minutes === 0) {
    return new Intl.DateTimeFormat(locale, { hour: 'numeric', hourCycle: cycle, timeZone }).format(
      date
    );
  }
  return formatTime(date, { timeZone, cycle, locale });
}

/** The zone a moment-timezone moment is in, for formatTime's `timeZone`. */
export function momentZone(m: { tz?: () => string | undefined }): string | undefined {
  return typeof m.tz === 'function' ? m.tz() || undefined : undefined;
}

/** A date without the time: "Nov 1, 2023". */
export function formatDate(
  date: Date,
  {
    style = 'medium',
    timeZone,
    locale = appLocale(),
  }: { style?: 'short' | 'medium' | 'long' | 'full'; timeZone?: string; locale?: string } = {}
) {
  return new Intl.DateTimeFormat(locale, { dateStyle: style, timeZone }).format(date);
}

/** A date and time: "Nov 1, 2023, 2:05 PM". */
export function formatDateTime(
  date: Date,
  {
    timeZone,
    locale = appLocale(),
    cycle = hourCycle(configGet(USE_24_HOUR_KEY), locale),
  }: { timeZone?: string; cycle?: HourCycle; locale?: string } = {}
) {
  return new Intl.DateTimeFormat(locale, quoteDateOptions(cycle, timeZone)).format(date);
}

function quoteDateOptions(cycle: HourCycle, timeZone?: string): Intl.DateTimeFormatOptions {
  return { dateStyle: 'medium', timeStyle: 'short', hourCycle: cycle, timeZone };
}

/** The date in a reply's attribution line and a forward's header. */
export function formatQuoteDate(date: Date, opts: { cycle?: HourCycle; locale?: string } = {}) {
  return formatDateTime(date, opts);
}

/** The settings value `composing.quoteTexts` the engine builds quotes with. */
export interface QuoteTexts {
  // "{date}" and "{from}" are filled in.
  attribution: string;
  forwardedMessage: string;
  from: string;
  subject: string;
  date: string;
  to: string;
  cc: string;
  // Literals and the fields {month} (a name from `months`), {monthNum}, {month2}, {day},
  // {day2}, {year}, {year2}, {hour}, {hour2}, {minute}, {dayPeriod} (from `dayPeriods`).
  dateTime: string;
  months: string[];
  dayPeriods: [string, string];
}

export const QUOTE_TEXTS_KEY = 'composing.quoteTexts';

export function attributionTemplate(t: (en: string) => string) {
  return t('On %1$@, %2$@ wrote:').replace('%1$@', '{date}').replace('%2$@', '{from}');
}

// Fields whose rendering is told apart by formatting 2001-01-01 01:05 and 2001-12-11 13:05.
function patternFor(locale: string, cycle: HourCycle): string {
  const fmt = new Intl.DateTimeFormat(locale, quoteDateOptions(cycle, 'UTC'));
  const early = fmt.formatToParts(new Date(Date.UTC(2001, 0, 1, 1, 5)));
  return early
    .map((part) => {
      const { type, value } = part;
      switch (type) {
        case 'literal':
          return value.replace(/[{}]/g, '');
        case 'month':
          if (/^\d+$/.test(value)) return value.length === 2 ? '{month2}' : '{monthNum}';
          return '{month}';
        case 'day':
          return value.length === 2 ? '{day2}' : '{day}';
        case 'year':
          return value.length === 2 ? '{year2}' : '{year}';
        case 'hour':
          return value.length === 2 ? '{hour2}' : '{hour}';
        case 'minute':
          return '{minute}';
        case 'dayPeriod':
          return '{dayPeriod}';
        default:
          return value;
      }
    })
    .join('');
}

export function quoteTexts(
  language: string | null | undefined,
  cycle: HourCycle,
  locale: string = language || appLocale()
): QuoteTexts {
  const t = localizerFor(language);
  const months: string[] = [];
  const monthFmt = new Intl.DateTimeFormat(locale, quoteDateOptions(cycle, 'UTC'));
  for (let m = 0; m < 12; m++) {
    const parts = monthFmt.formatToParts(new Date(Date.UTC(2001, m, 15, 12)));
    months.push((parts.find((p) => p.type === 'month') || { value: String(m + 1) }).value);
  }
  const period = (hour: number) => {
    const parts = new Intl.DateTimeFormat(locale, {
      hour: 'numeric',
      hourCycle: 'h12',
      timeZone: 'UTC',
    }).formatToParts(new Date(Date.UTC(2001, 0, 1, hour)));
    return (parts.find((p) => p.type === 'dayPeriod') || { value: hour < 12 ? 'AM' : 'PM' }).value;
  };
  return {
    attribution: attributionTemplate(t),
    forwardedMessage: t('Forwarded Message'),
    from: t('From'),
    subject: t('Subject'),
    date: t('Date'),
    to: t('To'),
    cc: t('Cc'),
    dateTime: patternFor(locale, cycle),
    months,
    dayPeriods: [period(1), period(13)],
  };
}
