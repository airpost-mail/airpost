import { AccountStore } from '../stores/account';
import { Calendar } from '../models/calendar';
import * as Utils from '../models/utils';
import RegExpUtils from '../../utils/regexp';

type ICAL = typeof import('ical.js').default;
type ICALComponent = InstanceType<ICAL['Component']>;
type ICALProperty = InstanceType<ICAL['Property']>;
type ICALEvent = InstanceType<ICAL['Event']>;

let ICALModule: ICAL | null = null;

// ical.js is large, so it loads on first use.
function loadICAL(): ICAL {
  if (!ICALModule) {
    ICALModule = require('ical.js') as ICAL;
  }
  return ICALModule;
}

// jCal (RFC 7265): a component is [name, properties, subcomponents], a property is
// [name, parameters, value type, ...values].
type JCalProperty = [string, { [param: string]: unknown }, string, ...unknown[]];
type JCalComponent = [string, JCalProperty[], JCalComponent[]];

export type ICSParticipantStatus =
  | 'NEEDS-ACTION'
  | 'ACCEPTED'
  | 'DECLINED'
  | 'TENTATIVE'
  | 'DELEGATED'
  | 'COMPLETED'
  | 'IN-PROCESS';

export interface ICSParticipant {
  email: string | null;
  role: 'CHAIR' | 'REQ-PARTICIPANT' | 'OPT-PARTICIPANT' | 'NON-PARTICIPANT';
  status: ICSParticipantStatus;
  component: ICALProperty;
}

function fixJCalDatesWithoutTimes(jCal: JCalComponent) {
  jCal[1].forEach((property) => {
    if (
      property[0] === 'dtstart' ||
      property[0] === 'dtend' ||
      property[0] === 'exdate' ||
      property[0] === 'rdate'
    ) {
      const value = property[3];
      if (
        !property[1].value &&
        property[2] === 'date-time' &&
        typeof value === 'string' &&
        /T::$/.test(value)
      ) {
        property[2] = 'date';
        property[3] = value.replace(/T::$/, '');
      }
    }
  });
  jCal[2].forEach(fixJCalDatesWithoutTimes);
}

export function parseICSString(ics: string) {
  const ICAL = loadICAL();
  const jcalData: JCalComponent = ICAL.parse(ics);

  // workaround https://github.com/mozilla-comm/ical.js/issues/186
  fixJCalDatesWithoutTimes(jcalData);

  const root = new ICAL.Component(jcalData);
  // Before ICAL.Event: relating the exceptions reads each RECURRENCE-ID, and a value read once
  // keeps the zone it was read with.
  registerTimezones(root);
  const event = new ICAL.Event(
    root.name === 'vevent' ? root : root.getFirstSubcomponent('vevent') || undefined
  );
  return { root, event };
}

/**
 * Registers a VCALENDAR's VTIMEZONEs with the ICAL.js TimezoneService, so `toJSDate()` on a
 * TZID-relative time gives the instant it names. This is for showing an emailed invitation: the
 * calendar's occurrences and every edit come from the engine (events.*), which also reads a TZID
 * a file has no VTIMEZONE for (RFC 7809) by its IANA or Windows name.
 */
function registerTimezones(vcalendar: ICALComponent): void {
  const ICAL = loadICAL();
  for (const vtz of vcalendar.getAllSubcomponents('vtimezone')) {
    // The registry is process-wide and ical.js seeds these three names with UTC itself, so a file
    // redefining one would move every time read through that name for the rest of the session.
    if (['UTC', 'GMT', 'Z'].includes(String(vtz.getFirstPropertyValue('tzid')))) continue;
    ICAL.TimezoneService.register(vtz);
  }
}

export function emailFromParticipantURI(uri: string): string | null {
  if (!uri) {
    return null;
  }

  // Normalize to lowercase for comparison
  const uriLower = uri.toLowerCase();

  // Handle mailto: URI format (most common)
  // e.g., "mailto:user@example.com" or "MAILTO:user@example.com"
  if (uriLower.startsWith('mailto:')) {
    const email = uri.slice(7).toLowerCase(); // preserve original then lowercase
    if (email.includes('@')) {
      return email;
    }
    return null;
  }

  // Handle bare email addresses (no mailto: prefix)
  // Some calendar systems just use "user@example.com" directly
  // Use RegExpUtils.emailRegex which supports international characters,
  // and verify the match covers the entire string to reject malformed inputs
  const emailRegex = RegExpUtils.emailRegex();
  const bareMatch = emailRegex.exec(uri);
  if (bareMatch && bareMatch.index === 0 && bareMatch[0].length === uri.length) {
    return uri.toLowerCase();
  }

  // Try to extract an email pattern from the string as a last resort.
  // This handles edge cases like "invalid:user@example.com" or other
  // non-standard formats where the email is embedded in the string.
  emailRegex.lastIndex = 0; // Reset regex state since it has the 'g' flag
  const embeddedMatch = emailRegex.exec(uri);
  if (embeddedMatch) {
    return embeddedMatch[0].toLowerCase();
  }

  return null;
}

export function cleanParticipants(icsEvent: ICALEvent): ICSParticipant[] {
  return icsEvent.attendees.map((a) => ({
    component: a,
    status: (a.getParameter('partstat') || 'NEEDS-ACTION') as ICSParticipantStatus,
    role: (a.getParameter('role') || 'REQ-PARTICIPANT') as ICSParticipant['role'],
    email:
      a
        .getValues()
        .map(emailFromParticipantURI)
        .find((v) => !!v) || null,
  }));
}

export function selfParticipant(
  icsEvent: ICALEvent,
  accountId: string
): (ICSParticipant & { email: string }) | undefined {
  const me = cleanParticipants(icsEvent).find((a): a is ICSParticipant & { email: string } => {
    if (!a.email) return false;
    const acct = AccountStore.accountForEmail(a.email);
    return !!acct && acct.id === accountId;
  });
  return me;
}

/**
 * DAV:owner (RFC 3744 section 5.1) when the server gave one. Otherwise the calendar is ours when
 * it is named after one of the account's addresses, as Google and Fastmail name the default one.
 */
export function isOwnCalendar(calendar: Calendar, addresses: string[]): boolean {
  if (calendar.ownership === 'mine') return true;
  if (calendar.ownership === 'other') return false;
  return addresses.some((address) => Utils.emailIsEquivalent(calendar.name, address));
}

/**
 * True only when the server named an owner and it was not us. Unlike `!isOwnCalendar`, a
 * calendar the server said nothing about is not ruled out.
 */
export function isSomeoneElsesCalendar(calendar: Calendar | undefined | null): boolean {
  return !!calendar && calendar.ownership === 'other';
}
