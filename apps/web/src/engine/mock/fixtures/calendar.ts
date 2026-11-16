// Calendars and events for the mock, relative to the fixture clock. Events are single
// occurrences: recurrence expansion moves to the engine with DCP stage 18.
import type { CalendarJSON, EventJSON } from '../../types';

const calendars = (gmail: string, imap: string): CalendarJSON[] => [
  { id: 'cal-g-personal', aid: gmail, v: 1, __cls: 'Calendar', name: 'Personal', color: '#3b6fe0' },
  { id: 'cal-g-family', aid: gmail, v: 1, __cls: 'Calendar', name: 'Family', color: '#2f9e6b' },
  { id: 'cal-i-work', aid: imap, v: 1, __cls: 'Calendar', name: 'Work', color: '#d9822b' },
];

/** [calendar, day offset, start hour (null: all day), minutes, title, location] */
const EVENTS: [string, number, number | null, number, string, string][] = [
  ['cal-i-work', -2, 9, 30, 'Sprint review', 'Room 2'],
  ['cal-i-work', 0, 10, 30, 'Stand-up', 'Video call'],
  ['cal-g-personal', 0, 13, 60, 'Lunch with Ines', 'Café Lisboa'],
  ['cal-i-work', 1, 15, 60, 'Design critique', 'Room 4'],
  ['cal-g-family', 2, null, 0, "Marta's birthday", ''],
  ['cal-i-work', 3, 11, 90, 'Quarterly planning', 'Board room'],
  ['cal-g-personal', 4, 18, 90, 'Climbing', 'Bloco Gym'],
  ['cal-i-work', 7, 10, 30, 'Stand-up', 'Video call'],
  ['cal-g-family', 9, 19, 120, 'Dinner at home', ''],
  ['cal-i-work', 12, 14, 60, 'Customer call: Northwind', 'Video call'],
];

const pad = (n: number) => String(n).padStart(2, '0');

function icsTime(t: number, allDay: boolean): string {
  const d = new Date(t * 1000);
  const date = `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}`;
  if (allDay) return date;
  return `${date}T${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}00Z`;
}

function ics(
  uid: string,
  start: number,
  end: number,
  allDay: boolean,
  title: string,
  where: string,
) {
  const prop = (name: string, t: number) =>
    allDay ? `${name};VALUE=DATE:${icsTime(t, true)}` : `${name}:${icsTime(t, false)}`;
  return [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Airpost//Mock//EN',
    'BEGIN:VEVENT',
    `UID:${uid}`,
    prop('DTSTART', start),
    prop('DTEND', end),
    `SUMMARY:${title.replace(/[,;]/g, (c) => `\\${c}`)}`,
    ...(where ? [`LOCATION:${where.replace(/[,;]/g, (c) => `\\${c}`)}`] : []),
    'END:VEVENT',
    'END:VCALENDAR',
  ].join('\r\n');
}

export function buildCalendars(
  now: number,
  ids: { gmail: string; imap: string },
): { calendars: CalendarJSON[]; events: EventJSON[] } {
  const all = calendars(ids.gmail, ids.imap);
  const midnight = new Date(now * 1000);
  midnight.setUTCHours(0, 0, 0, 0);
  const day0 = midnight.getTime() / 1000;
  const events = EVENTS.map(([cid, day, hour, minutes, title, where], i): EventJSON => {
    const cal = all.find((c) => c.id === cid)!;
    const allDay = hour === null;
    const start = day0 + day * 86400 + (hour ?? 0) * 3600;
    const end = allDay ? start + 86400 : start + minutes * 60;
    const uid = `mock-event-${i + 1}@airpost.local`;
    return {
      id: `ev-${i + 1}`,
      aid: cal.aid,
      v: 1,
      __cls: 'Event',
      cid,
      ics: ics(uid, start, end, allDay, title, where),
      icsuid: uid,
      rid: '',
      status: 'CONFIRMED',
      rs: start,
      re: end,
    };
  });
  return { calendars: all, events };
}
