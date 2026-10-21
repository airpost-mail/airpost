import moment, { Moment } from 'moment-timezone';
import { appLocale, hourCycle } from '../intl/date-format';

// Init locale for moment
moment.locale(navigator.language);

/**
 * The current IANA name of a zone: America/Indiana/Indianapolis for America/Indianapolis,
 * Asia/Kolkata for Asia/Calcutta. The machine reports whatever name its zone is configured under,
 * and every TZID this app writes for local time uses it. SabreDAV 4.7 (Baikal 0.10.1, measured
 * 2026-10-09) resolves only current names and reads any other TZID as UTC in free/busy, despite
 * the VTIMEZONE beside it.
 *
 * Current means listed in tzdata's zone.tab, as moment-timezone's metadata carries it. CLDR's
 * names for the same location are tried first: tzdata links zones of different countries that
 * share rules (Africa/Asmara to Africa/Nairobi), and only CLDR keeps them apart. Its list predates
 * a few renames (Europe/Kyiv), which tzdata's links then supply. A name with no current zone
 * (UTC, Etc/GMT+5) is returned unchanged.
 */
export function canonicalZoneName(name: string): string {
  const current = require('moment-timezone/data/meta/latest.json').zones;
  const isCurrent = (zone: string) => !!current[zone];
  const sameLocation: string[] = require('windows-iana').findAlias(name) || [];
  return sameLocation.find(isCurrent) || linkedZoneNames(name).find(isCurrent) || name;
}

// The name and every name tzdata links to it, directly or through another link.
function linkedZoneNames(name: string): string[] {
  const pairs: string[][] = require('moment-timezone/data/packed/latest.json').links.map(
    (link: string) => link.split('|')
  );
  const group = new Set([name]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const [a, b] of pairs) {
      if (group.has(a) !== group.has(b)) {
        group.add(a);
        group.add(b);
        grew = true;
      }
    }
  }
  return [...group];
}

// Initialise moment timezone
const tz = canonicalZoneName(moment.tz.guess());
if (!tz) {
  console.error('DateUtils: TimeZone could not be determined. This should not happen!');
}

const yearRegex = / ?YY(YY)?/;

const Hours = {
  Morning: 9,
  Evening: 20,
  Midnight: 24,
};

const Days = {
  // The value for next monday and next weekend varies depending if the current
  // day is saturday or sunday. See http://momentjs.com/docs/#/get-set/day/
  NextMonday: (day: number) => (day === 0 ? 1 : 8),
  ThisWeekend: (day: number) => (day === 6 ? 13 : 6),
};

function oclock(momentDate: Moment) {
  return momentDate.minute(0).second(0);
}

function morning(momentDate: Moment, morningHour = Hours.Morning) {
  return oclock(momentDate.hour(morningHour));
}

function evening(momentDate: Moment, eveningHour = Hours.Evening) {
  return oclock(momentDate.hour(eveningHour));
}

function midnight(momentDate: Moment, midnightHour = Hours.Midnight) {
  return oclock(momentDate.hour(midnightHour));
}

function isPastDate(
  inputDateObj: { [component: string]: number },
  currentDate: Date | Moment | string | undefined
) {
  const inputMoment = moment({ ...inputDateObj, month: inputDateObj.month - 1 });
  const currentMoment = moment(currentDate);

  return inputMoment.isBefore(currentMoment);
}

// The parts of chrono-node this file uses. Its typings make `knownValues` and `impliedValues`
// private, and the refiners below read and adjust them.
interface ChronoComponents {
  knownValues: { [component: string]: number };
  impliedValues: { [component: string]: number };
  isCertain(component: string): boolean;
  imply(component: string, value: number): unknown;
}

interface ChronoResult {
  text: string;
  ref?: Date;
  start: ChronoComponents;
  end?: ChronoComponents;
}

interface ChronoParser {
  parse(text: string): ChronoResult[];
  parseDate(text: string): Date | null;
  refiners: Array<{ refine: (text: unknown, results: ChronoResult[]) => ChronoResult[] }>;
}

interface ChronoModule {
  casual: ChronoParser & { clone(): ChronoParser };
  parse(text: string): ChronoResult[];
}

let _chronoPast: ChronoParser | null = null;
let _chronoFuture: ChronoParser | null = null;
let _chrono: ChronoModule | null = null;

function getChrono(): ChronoModule {
  const chrono: ChronoModule = _chrono || require('chrono-node'); //eslint-disable-line
  _chrono = chrono;
  return chrono;
}

function getChronoFuture(): ChronoParser {
  if (_chronoFuture) {
    return _chronoFuture;
  }

  const chrono = getChrono();

  _chronoFuture = chrono.casual.clone();
  _chronoFuture.refiners.push({
    refine: (text, results) => {
      results.forEach((result) => {
        const current = Object.assign({}, result.start.knownValues, result.start.impliedValues);

        if (result.start.isCertain('weekday') && !result.start.isCertain('day')) {
          if (isPastDate(current, result.ref)) {
            result.start.imply('day', result.start.impliedValues.day + 7);
          }
        }

        if (result.start.isCertain('day') && !result.start.isCertain('month')) {
          if (isPastDate(current, result.ref)) {
            result.start.imply('month', result.start.impliedValues.month + 1);
          }
        }
        if (result.start.isCertain('month') && !result.start.isCertain('year')) {
          if (isPastDate(current, result.ref)) {
            result.start.imply('year', result.start.impliedValues.year + 1);
          }
        }
      });
      return results;
    },
  });
  return _chronoFuture;
}

function getChronoPast(): ChronoParser {
  if (_chronoPast) {
    return _chronoPast;
  }

  const chrono = getChrono();

  _chronoPast = chrono.casual.clone();
  _chronoPast.refiners.push({
    refine: (text, results) => {
      results.forEach((result) => {
        const current = Object.assign({}, result.start.knownValues, result.start.impliedValues);

        if (result.start.isCertain('weekday') && !result.start.isCertain('day')) {
          if (!isPastDate(current, result.ref)) {
            result.start.imply('day', result.start.impliedValues.day - 7);
          }
        }

        if (result.start.isCertain('day') && !result.start.isCertain('month')) {
          if (!isPastDate(current, result.ref)) {
            result.start.imply('month', result.start.impliedValues.month - 1);
          }
        }
        if (result.start.isCertain('month') && !result.start.isCertain('year')) {
          if (!isPastDate(current, result.ref)) {
            result.start.imply('year', result.start.impliedValues.year - 1);
          }
        }
      });
      return results;
    },
  });
  return _chronoPast;
}

function expandDateLikeString(dateLikeString: string) {
  // Short format: 123 => 1:23 or 1234 => 12:34
  if (/^\d{3,4}$/.test(dateLikeString)) {
    const len = dateLikeString.length;
    dateLikeString = dateLikeString.slice(0, len - 2) + ':' + dateLikeString.slice(len - 2); // Insert colon
  }

  // Short format: 2h
  const hours = /^(\d+)h$/.exec(dateLikeString);
  if (hours) {
    return `${hours[1]} hours`;
  }

  // Short format: 2d
  const days = /^(\d+)d$/.exec(dateLikeString);
  if (days) {
    return `${days[1]} days`;
  }

  // Short format: 2w
  const weeks = /^(\d+)w$/.exec(dateLikeString);
  if (weeks) {
    return `${weeks[1]} weeks`;
  }

  // Short format: 2m, 2mo
  const months = /^(\d+)mo?$/.exec(dateLikeString);
  if (months) {
    return `${months[1]} months`;
  }

  // Short format: t, to, tom => tomorrow
  if (['t', 'to', 'tom', 'tom '].indexOf(dateLikeString) >= 0) {
    return `tomorrow morning`;
  }

  // Short format: nw, next week => next week
  if (['nw', 'next week'].indexOf(dateLikeString) >= 0) {
    return `next Monday`;
  }

  if (dateLikeString.indexOf('tom ') === 0) {
    return dateLikeString.replace('tom ', 'tomorrow ');
  }

  return dateLikeString; // Return original string if no match
}

function format(momentDate: Moment, formatString?: string): string;
function format(momentDate: Moment | null | undefined, formatString?: string): string | null;
function format(momentDate: Moment | null | undefined, formatString?: string) {
  if (!momentDate) return null;
  return momentDate.format(formatString);
}

function utc(momentDate: Moment): Moment;
function utc(momentDate: Moment | null | undefined): Moment | null;
function utc(momentDate: Moment | null | undefined) {
  if (!momentDate) return null;
  return momentDate.utc();
}

const DateUtils = {
  // Localized format: ddd, MMM D, YYYY h:mmA
  DATE_FORMAT_LONG: 'llll',

  DATE_FORMAT_LONG_NO_YEAR: moment.localeData().longDateFormat('llll').replace(yearRegex, ''),

  // Localized format: MMM D, h:mmA
  DATE_FORMAT_SHORT: moment.localeData().longDateFormat('lll').replace(yearRegex, ''),

  DATE_FORMAT_llll_NO_TIME: moment
    .localeData()
    .longDateFormat('llll')
    .replace(/h:mm/, '')
    .replace(' A', ''),

  DATE_FORMAT_LLLL_NO_TIME: moment
    .localeData()
    .longDateFormat('LLLL')
    .replace(/h:mm/, '')
    .replace(' A', ''),

  timeZone: tz,

  format,

  utc,

  minutesFromNow(minutes: number, now = moment()) {
    return now.add(minutes, 'minutes');
  },

  hoursFromNow(hours: number, now = moment()) {
    return now.add(hours, 'hours');
  },

  in1Hour() {
    return DateUtils.minutesFromNow(60);
  },

  in2Hours() {
    return DateUtils.minutesFromNow(120);
  },

  laterToday(now = moment()) {
    return oclock(now.add(3, 'hours'));
  },

  tonight(now = moment()) {
    if (now.hour() >= Hours.Evening) {
      return midnight(now);
    }
    return evening(now);
  },

  tomorrow(now = moment()) {
    return morning(now.add(1, 'day'));
  },

  tomorrowEvening(now = moment()) {
    return evening(now.add(1, 'day'));
  },

  thisWeekend(now = moment()) {
    return morning(now.day(Days.ThisWeekend(now.day())));
  },

  weeksFromNow(weeks: number, now = moment()) {
    return now.add(weeks, 'weeks');
  },

  nextWeek(now = moment()) {
    return morning(now.day(Days.NextMonday(now.day())));
  },

  monthsFromNow(months: number, now = moment()) {
    return now.add(months, 'months');
  },

  nextMonth(now = moment()) {
    return morning(now.add(1, 'month').date(1));
  },

  getChronoPast,

  parseDateString(dateLikeString: string): {
    leftoverText: string;
    start: Moment;
    end: Moment;
  } {
    const parsed = getChrono().parse(dateLikeString);
    const gotTime = { start: false, end: false };
    const gotDay = { start: false, end: false };
    const now = moment();
    const results = { start: moment(now), end: moment(now), leftoverText: dateLikeString };
    for (const item of parsed) {
      for (const val of ['start', 'end'] as const) {
        const components = item[val];
        if (!components) {
          continue;
        }
        const { day: knownDay, weekday: knownWeekday, hour: knownHour } = components.knownValues;
        const { year, month, day, hour, minute } = Object.assign(
          components.knownValues,
          components.impliedValues
        );
        if (!gotTime[val] && knownHour) {
          gotTime[val] = true;
          results[val].minute(minute);
          results[val].hour(hour);

          if (!gotDay[val]) {
            results[val].date(day);
            results[val].month(month - 1); // moment zero-indexes month
            results[val].year(year);
          }

          results.leftoverText = results.leftoverText.replace(item.text, '');
        }
        if (!gotDay[val] && (knownDay || knownWeekday)) {
          gotDay[val] = true;
          results[val].year(year);
          results[val].month(month - 1); // moment zero-indexes month
          results[val].date(day);

          if (!gotTime[val]) {
            results[val].hour(hour);
            results[val].minute(minute);
          }

          results.leftoverText = results.leftoverText.replace(item.text, '');
        }
      }
    }

    // Make the event a default 1 hour long if it looks like the end date
    // wasn't assigned, or if it's before the start date.
    if (results.end.valueOf() === now.valueOf() || results.end <= results.start) {
      results.end = moment(results.start);
      results.end.hour(results.end.hour() + 1);
    }

    return results;
  },

  /**
   * Can take almost any string.
   * e.g. "Next Monday at 2pm"
   * @param {string} dateLikeString - a string representing a date.
   * @return {moment} - moment object representing date
   */
  futureDateFromString(dateLikeString: string) {
    const expanded = expandDateLikeString(dateLikeString);
    const date = getChronoFuture().parseDate(expanded);
    if (!date) {
      return null;
    }
    const inThePast = date.valueOf() < Date.now();
    if (inThePast) {
      return null;
    }
    return moment(date);
  },

  /**
   * Return a formatting string for displaying time
   *
   * @param {Date} opts - Object with different properties for customising output
   * @return {String} The format string based on syntax used by Moment.js
   *
   * seconds, upperCase and timeZone are the supported extra options to the format string.
   * Checks whether or not to use 24 hour time format.
   */
  getTimeFormat(opts?: { seconds?: boolean; upperCase?: boolean; timeZone?: boolean } | null) {
    const use24HourClock = hourCycle() === 'h23';
    let timeFormat = use24HourClock ? 'HH:mm' : 'h:mm';

    if (opts && opts.seconds) {
      timeFormat += ':ss';
    }

    // Append meridian if not using 24 hour clock
    if (!use24HourClock) {
      if (opts && opts.upperCase) {
        timeFormat += ' A';
      } else {
        timeFormat += ' a';
      }
    }

    if (opts && opts.timeZone) {
      timeFormat += ' z';
    }

    return timeFormat;
  },

  /**
   * Return a short format date/time
   *
   * @param {Date} datetime - Timestamp
   * @return {String} Formated date/time
   *
   * The returned date/time format depends on how long ago the timestamp is.
   */
  shortTimeString(datetime: Date) {
    const now = moment();
    const diff = now.diff(datetime, 'days', true);
    const isSameDay = now.isSame(datetime, 'days');
    const opts: Intl.DateTimeFormatOptions = {
      hourCycle: hourCycle(),
    };

    if (diff <= 1 && isSameDay) {
      // Time if less than 1 day old
      opts.hour = 'numeric';
      opts.minute = '2-digit';
    } else if (diff < 5 && !isSameDay) {
      // Weekday with time if up to 2 days ago
      //opts.month = 'short';
      //opts.day = 'numeric';
      opts.weekday = 'short';
      opts.hour = 'numeric';
      opts.minute = '2-digit';
    } else {
      if (diff < 365) {
        // Month and day up to 1 year old
        opts.month = 'short';
        opts.day = 'numeric';
      } else {
        // Month, day and year if over a year old
        opts.year = 'numeric';
        opts.month = 'short';
        opts.day = 'numeric';
      }
      return datetime.toLocaleDateString(appLocale(), opts);
    }

    return datetime.toLocaleTimeString(appLocale(), opts);
  },

  /**
   * Return a medium format date/time
   *
   * @param {Date} datetime - Timestamp
   * @return {String} Formated date/time
   */
  mediumTimeString(datetime: Date) {
    return datetime.toLocaleTimeString(appLocale(), {
      hourCycle: hourCycle(),
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      second: undefined,
    });
  },

  /**
   * Return a long format date/time
   *
   * @param {Date} datetime - Timestamp
   * @return {String} Formated date/time
   */
  fullTimeString(datetime: Date) {
    // ISSUE: this does drop ordinal. There is this:
    // -> new Intl.PluralRules(LOCALE, { type: "ordinal" }).select(dateTime.getDay())
    // which may work with the below regex, though localisation is required
    // `(?<!\d)${dateTime.getDay()}(?!\d)` replace `$1${localise(ordinal)}`

    return datetime.toLocaleTimeString(appLocale(), {
      hourCycle: hourCycle(),
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      weekday: 'long',
      second: undefined,
    });
  },
};

export default DateUtils;
