import { describe, expect, it } from 'vitest';
import type { EventJSON } from '../types';
import { occurrenceOf } from './events';

const event = (ics: string, over: Partial<EventJSON> = {}): EventJSON => ({
  id: 'e1',
  aid: 'a',
  v: 1,
  __cls: 'Event',
  cid: 'c',
  ics,
  icsuid: 'u',
  rid: '',
  rs: 20379 * 86400,
  re: 20380 * 86400,
  ...over,
});

describe('occurrenceOf', () => {
  it('reads unfolded, escaped fields and all-day dates', () => {
    const o = occurrenceOf(
      event(
        [
          'BEGIN:VCALENDAR',
          'BEGIN:VTIMEZONE',
          'SUMMARY:not this',
          'END:VTIMEZONE',
          'BEGIN:VEVENT',
          'DTSTART;VALUE=DATE:20261018',
          'SUMMARY:Lunch\\, then a ',
          ' walk',
          'LOCATION:Café\\; terrace',
          'RRULE:FREQ=WEEKLY',
          'END:VEVENT',
          'END:VCALENDAR',
        ].join('\r\n'),
      ),
    );
    expect(o).toMatchObject({
      title: 'Lunch, then a walk',
      location: 'Café; terrace',
      isAllDay: true,
      isRecurring: true,
      startDate: 20379,
      endDate: 20379,
    });
    expect(o.start).toBeUndefined();
  });

  it('copes with a timed event and a missing title', () => {
    const o = occurrenceOf(
      event('BEGIN:VEVENT\nDTSTART;TZID=Europe/Lisbon:20261018T100000\nEND:VEVENT', {
        rs: 100,
        re: 200,
      }),
    );
    expect(o).toMatchObject({ title: '', isAllDay: false, isRecurring: false, start: 100 });
  });
});
