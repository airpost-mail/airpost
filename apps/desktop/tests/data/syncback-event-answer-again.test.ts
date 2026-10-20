import { DatabaseStore, Event } from 'mailspring-exports';
import { SyncbackEventTask } from '../../src/data/tasks/syncback-event';
import { fakePimEngine, refusal } from '../support/fake-pim-engine';

const ME = 'brian@example.com';

const ics = ({ start = '20261022T160000Z', location = 'Cafe', partstat = 'NEEDS-ACTION' } = {}) =>
  [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Test//Test//EN',
    'BEGIN:VEVENT',
    'UID:lunch@test',
    `DTSTART:${start}`,
    'DTEND:20261022T173000Z',
    'SUMMARY:Lunch',
    `LOCATION:${location}`,
    'DTSTAMP:20261001T000000Z',
    'ORGANIZER:mailto:ryan@example.com',
    'ATTENDEE;PARTSTAT=ACCEPTED:mailto:ryan@example.com',
    `ATTENDEE;PARTSTAT=${partstat}:mailto:${ME}`,
    'END:VEVENT',
    'END:VCALENDAR',
  ].join('\r\n');

const event = (icsText: string, id = 'event-1') =>
  new Event({ id, accountId: 'acct-1', calendarId: 'cal-1', ics: icsText } as any);

describe('an answer refused because our copy changed on the server', function () {
  let listener: (record: any) => void;
  let unlisten: jasmine.Spy;
  let engine: ReturnType<typeof fakePimEngine>;
  let dialog: jasmine.Spy;

  // Our ACCEPTED written over the copy we had, which the organizer has since moved.
  const answerTask = (answer: any = { email: ME, status: 'ACCEPTED' }) =>
    SyncbackEventTask.forAnswering({ event: event(ics({ partstat: 'ACCEPTED' })), answer });

  // What the calendar sync stores: the organizer's new time and place, still awaiting us.
  const moved = () => event(ics({ start: '20261022T170000Z', location: "Ruthie's" }));

  const stored = (events: Event[], type = 'persist') => ({
    type,
    objectClass: 'Event',
    objects: events,
  });

  beforeEach(function () {
    unlisten = jasmine.createSpy('unlisten');
    spyOn(DatabaseStore, 'listen').andCallFake((fn) => {
      listener = fn;
      return unlisten;
    });
    spyOn(AppEnv.mailsyncBridge, 'sendSyncCalendarNow');
    engine = fakePimEngine();
    engine.answers['events.rsvp'] = (params) => ({
      email: ME,
      write: { kind: 'update', eventId: params.eventId, taskId: 't-2' },
    });
    dialog = spyOn(AppEnv, 'showErrorDialog');
  });

  it('re-reads the calendar and makes the answer again on the copy the server has now', async function () {
    const done = answerTask().onError({ key: 'etag-conflict', debuginfo: '' });
    expect(AppEnv.mailsyncBridge.sendSyncCalendarNow).toHaveBeenCalledWith('acct-1');
    listener(stored([moved()]));
    await done;

    const [params] = engine.callsTo('events.rsvp');
    expect(params.eventId).toBe('event-1');
    expect(params.status).toBe('ACCEPTED');
    // Recorded on our copy only: the organizer had the REPLY already.
    expect(params.reply).toBe(false);
    expect(params.retried).toBe(true);
    expect(params.ics).toBeUndefined();
    expect(dialog).not.toHaveBeenCalled();
    expect(unlisten.callCount).toBe(1);
  });

  it('answers only the occurrence it was asked about', async function () {
    const occurrenceIcs = 'BEGIN:VCALENDAR\r\nEND:VCALENDAR';
    const done = answerTask({ email: ME, status: 'DECLINED', occurrenceIcs }).onError({
      key: 'etag-conflict',
      debuginfo: '',
    });
    listener(stored([moved()]));
    await done;
    expect(engine.callsTo('events.rsvp')[0].ics).toBe(occurrenceIcs);
  });

  it('waits for this event, not another one or a removal', async function () {
    const done = answerTask().onError({ key: 'etag-conflict', debuginfo: '' });
    listener(stored([event(ics(), 'event-2')]));
    listener(stored([moved()], 'unpersist'));
    listener({ type: 'persist', objectClass: 'Message', objects: [moved()] });
    expect(unlisten).not.toHaveBeenCalled();
    listener(stored([moved()]));
    await done;
    expect(engine.callsTo('events.rsvp').length).toBe(1);
  });

  it('shows the conflict when the calendar does not come back within 30 seconds', async function () {
    const done = answerTask().onError({ key: 'etag-conflict', debuginfo: '' });
    advanceClock(29 * 1000);
    expect(unlisten).not.toHaveBeenCalled();
    advanceClock(1000);
    await done;
    expect(unlisten).toHaveBeenCalled();
    expect(engine.calls.length).toBe(0);
    expect(dialog.mostRecentCall.args[0].message).toContain('changed by another client');
  });

  it('stops waiting once the copy arrives', async function () {
    const done = answerTask().onError({ key: 'etag-conflict', debuginfo: '' });
    listener(stored([moved()]));
    await done;
    advanceClock(60 * 1000);
    expect(unlisten.callCount).toBe(1);
  });

  it('shows the conflict when the new copy no longer lists us', async function () {
    engine.answers['events.rsvp'] = () => {
      throw refusal('not-invited');
    };
    const done = answerTask().onError({ key: 'etag-conflict', debuginfo: '' });
    listener(stored([event(ics().replace(`mailto:${ME}`, 'mailto:someone@example.com'))]));
    await done;
    expect(dialog).toHaveBeenCalled();
  });

  it('makes an answer again only once', async function () {
    await answerTask({ email: ME, status: 'ACCEPTED', retried: true }).onError({
      key: 'etag-conflict',
      debuginfo: '',
    });
    expect(AppEnv.mailsyncBridge.sendSyncCalendarNow).not.toHaveBeenCalled();
    expect(dialog).toHaveBeenCalled();
  });

  it('leaves an ordinary edit that conflicts to the user', async function () {
    await SyncbackEventTask.forUpdating({ event: event(ics()) }).onError({
      key: 'etag-conflict',
      debuginfo: '',
    });
    expect(AppEnv.mailsyncBridge.sendSyncCalendarNow).not.toHaveBeenCalled();
    expect(dialog).toHaveBeenCalled();
  });

  it('does not retry an answer refused for another reason', async function () {
    await answerTask().onError({ key: 'not-found', debuginfo: '' });
    expect(AppEnv.mailsyncBridge.sendSyncCalendarNow).not.toHaveBeenCalled();
    expect(dialog.mostRecentCall.args[0].message).toContain('no longer exists');
  });
});
