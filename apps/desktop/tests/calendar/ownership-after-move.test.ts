import DatabaseStore from '../../src/data/database/store';
import { Event } from '../../src/data/models/event';
import { Calendar } from '../../src/data/models/calendar';
import { MainCalendar } from '../../src/features/main-calendar/main-calendar';
import { TimedOccurrence } from '../../src/features/main-calendar/data-source';
import { fakePimEngine } from '../support/fake-pim-engine';

// Whether the meeting is ours is the engine's answer (crates/airpost-pim's calendar_addresses: a
// calendar's own group address organizes it); the selection takes the occurrence as re-expanded.

const START = Date.UTC(2026, 8, 22, 13, 0, 0) / 1000;
const QUARTER = 900;
const GROUP_ADDRESS = 'c_1a2b@group.calendar.google.com';

const stamp = (unix: number) =>
  new Date(unix * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+/, '');

// A meeting Google created on a secondary calendar: the calendar's own id is its ORGANIZER.
const eventAt = (start: number) =>
  new Event({
    id: 'team-sync',
    accountId: 'a',
    calendarId: 'team',
    icsuid: 'team-sync@test',
    recurrenceStart: start,
    recurrenceEnd: start + 1800,
    ics: [
      'BEGIN:VCALENDAR',
      'VERSION:2.0',
      'PRODID:-//Test//Test//EN',
      'BEGIN:VEVENT',
      'UID:team-sync@test',
      `DTSTART:${stamp(start)}`,
      `DTEND:${stamp(start + 1800)}`,
      `ORGANIZER;CN=Team:mailto:${GROUP_ADDRESS}`,
      'SUMMARY:Team sync',
      'DTSTAMP:20260901T000000Z',
      'END:VEVENT',
      'END:VCALENDAR',
    ].join('\r\n'),
  } as any);

describe('a meeting our own secondary calendar organizes, after it is moved', function () {
  it('is still ours once the move syncs back and the selection is refreshed', async function () {
    const engine = fakePimEngine();
    engine.answers['events.expand'] = () => ({
      occurrences: [
        {
          id: `team-sync-e${START + QUARTER}`,
          eventId: 'team-sync',
          accountId: 'a',
          calendarId: 'team',
          title: 'Team sync',
          isAllDay: false,
          isMine: true,
          organizer: { email: GROUP_ADDRESS },
          attendees: [],
          start: START + QUARTER,
          end: START + QUARTER + 1800,
        },
      ],
    });
    const calendar: any = new MainCalendar({} as any);
    calendar.setState = (update: object) => Object.assign(calendar.state, update);
    calendar.state.calendarsLoaded = true;
    calendar.state.calendars = [
      new Calendar({
        id: 'team',
        accountId: 'a',
        name: 'Team',
        ownership: 'mine',
        path: '/caldav/v2/c_1a2b%40group.calendar.google.com/events/',
      } as any),
    ];
    calendar.state.selectedEvents = [
      {
        id: `team-sync-e${START}`,
        accountId: 'a',
        calendarId: 'team',
        title: 'Team sync',
        isAllDay: false,
        isMine: true,
        organizer: { email: GROUP_ADDRESS },
        attendees: [],
        start: START,
        end: START + 1800,
      } as TimedOccurrence,
    ];
    spyOn(DatabaseStore, 'find').andCallFake(() => Promise.resolve(eventAt(START + QUARTER)));
    calendar._pendingMoves.set(`team-sync-e${START + QUARTER}`, {
      start: START + QUARTER,
      staleIcs: 'old',
    });
    calendar.state.selectedEvents = [
      { ...calendar.state.selectedEvents[0], id: `team-sync-e${START + QUARTER}` },
    ];

    await calendar._refreshSelectedEvents();

    expect(engine.callsTo('events.expand')[0].eventIds).toEqual(['team-sync']);
    const [selected] = calendar.state.selectedEvents;
    expect(selected.start).toBe(START + QUARTER);
    expect(selected.isMine).toBe(true);
  });
});
