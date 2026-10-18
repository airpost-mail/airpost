import { describe, expect, it } from 'vitest';
import type { EventJSON } from '@/engine';
import { summarizeEvent } from './ics';

const event = (ics: string, over: Partial<EventJSON> = {}): EventJSON => ({
  id: 'e1',
  aid: 'a',
  v: 1,
  __cls: 'Event',
  cid: 'c',
  ics,
  icsuid: 'u',
  rid: '',
  rs: 100,
  re: 200,
  ...over,
});

describe('summarizeEvent', () => {
  it('reads unfolded, escaped fields and all-day dates', () => {
    const s = summarizeEvent(
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
    expect(s).toMatchObject({
      title: 'Lunch, then a walk',
      location: 'Café; terrace',
      allDay: true,
      recurring: true,
      start: 100,
      end: 200,
    });
  });

  it('copes with a timed event and a missing title', () => {
    const s = summarizeEvent(
      event('BEGIN:VEVENT\nDTSTART;TZID=Europe/Lisbon:20261018T100000\nEND:VEVENT', {
        rid: 'x',
      }),
    );
    expect(s).toMatchObject({ title: '(no title)', allDay: false, recurring: false });
  });
});
