// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Calendar and contact editing on the engine (Feature.CalendarEditing and Feature.ContactEditing).
The engine expands occurrences and builds
the ICS and vCards; the calendar, events and contacts packages keep their UI and call these.

- `expandEvents`: `events.expand` in the machine's zone.
- `editEvent` and `createEvent`: `events.update` and `events.create` with `queue: false`, and the
  writes queued here, so UndoRedoStore sees them and SyncbackEventTask can undo them. An undo or
  redo re-sends its snapshot with `restore`, which the engine applies over the copy stored by
  then.
- `answerEvent`: `events.rsvp`, which the engine queues itself (an answer can't be undone).
- `getContact`, `saveContact`, `importContacts`, `exportContacts`: `contacts.*`.

A refusal comes back as an EngineConnectionError whose `data.key` names it (read-only,
not-invited, no-organizer, invalid-ics, invalid-params); `refusalKey` reads it.
*/
import EngineEvents, { EngineEvents as EngineEventsType } from './events';
import {
  ContactDetailsJSON,
  ContactsImportResult,
  EventPatch,
  EventScope,
  EventsCreateParams,
  EventsExpandParams,
  EventsExpandResult,
  EventsRsvpParams,
  EventsRsvpResult,
  EventsWriteResult,
  EventWriteJSON,
  Method,
} from '../protocol';
import { localTimeZone } from './drafts';
import * as Actions from '../../data/actions';
import DatabaseStore from '../../data/database/store';
import { Event } from '../../data/models/event';
import { Task } from '../../data/tasks/task';
import { SyncbackEventTask } from '../../data/tasks/syncback-event';
import { DestroyEventTask } from '../../data/tasks/destroy-event';

export function refusalKey(err: any): string | null {
  const key = err && err.data && err.data.key;
  return typeof key === 'string' ? key : null;
}

export function expandEvents(
  params: EventsExpandParams,
  engine: EngineEventsType = EngineEvents
): Promise<EventsExpandResult> {
  return engine.request<EventsExpandResult>(Method.EventsExpand, {
    tz: localTimeZone(),
    ...params,
  });
}

function snapshot(event: Event) {
  return {
    ics: event.ics,
    recurrenceStart: event.recurrenceStart,
    recurrenceEnd: event.recurrenceEnd,
  };
}

async function storedEvent(id: string, known?: Event): Promise<Event | null> {
  if (known && known.id === id) return known;
  return DatabaseStore.find<Event>(Event, id);
}

// The task that sends one of the engine's writes.
async function taskForWrite(
  write: EventWriteJSON,
  { accountId, event, description }: { accountId: string; event?: Event; description?: string }
): Promise<Task | null> {
  // tern-ipc EventWrite: a create names the calendar, an update or destroy the row.
  if (write.kind === 'create') {
    const { calendarId } = write;
    if (calendarId === undefined) return null;
    const created = new Event({ calendarId, accountId, ics: write.ics });
    return SyncbackEventTask.forCreating({ event: created, calendarId, accountId });
  }
  if (write.eventId === undefined) return null;
  const stored = await storedEvent(write.eventId, event);
  if (!stored) return null;
  if (write.kind === 'destroy') {
    return DestroyEventTask.forRemoving({ events: [stored] });
  }
  const updated = stored.clone();
  // An update always carries the event's new ICS.
  if (write.ics !== undefined) updated.ics = write.ics;
  return SyncbackEventTask.forUpdating({
    event: updated,
    undoData: snapshot(stored),
    description,
  });
}

/**
 * Changes an event: one occurrence (`this`, with `occurrenceStart`, the slot it fills), it and
 * the ones after (`following`), or the whole series (`all`). Resolves with the tasks queued.
 */
export async function editEvent(
  {
    event,
    patch,
    scope = 'all',
    occurrenceStart,
    description,
  }: {
    event: Event;
    patch: EventPatch;
    scope?: EventScope;
    occurrenceStart?: number;
    description?: string;
  },
  engine: EngineEventsType = EngineEvents
): Promise<Task[]> {
  const result = await engine.request<EventsWriteResult>(Method.EventsUpdate, {
    accountId: event.accountId,
    eventId: event.id,
    patch,
    scope,
    occurrenceStart,
    tz: localTimeZone(),
    queue: false,
  });
  const tasks: Task[] = [];
  for (const write of result.writes || []) {
    const task = await taskForWrite(write, { accountId: event.accountId, event, description });
    if (task) tasks.push(task);
  }
  if (tasks.length === 1) Actions.queueTask(tasks[0]);
  else if (tasks.length) Actions.queueTasks(tasks);
  return tasks;
}

/** Creates an event; resolves with the SyncbackEventTask queued for it. */
export async function createEvent(
  params: Omit<EventsCreateParams, 'queue' | 'tz'>,
  engine: EngineEventsType = EngineEvents
): Promise<{ task: SyncbackEventTask; icsuid: string | undefined }> {
  const result = await engine.request<EventsWriteResult>(Method.EventsCreate, {
    ...params,
    tz: localTimeZone(),
    queue: false,
  });
  const write = (result.writes || [])[0];
  const task = (await taskForWrite(write, { accountId: params.accountId })) as SyncbackEventTask;
  Actions.queueTask(task);
  return { task, icsuid: result.icsuid };
}

/** Answers an invitation, or with `counter` proposes another time; the engine queues the tasks. */
export function answerEvent(
  params: Omit<EventsRsvpParams, 'tz'>,
  engine: EngineEventsType = EngineEvents
): Promise<EventsRsvpResult> {
  return engine.request<EventsRsvpResult>(Method.EventsRsvp, { tz: localTimeZone(), ...params });
}

export function getContact(
  { accountId, contactId }: { accountId: string; contactId: string },
  engine: EngineEventsType = EngineEvents
): Promise<ContactDetailsJSON> {
  return engine.request<ContactDetailsJSON>(Method.ContactsGet, { accountId, contactId });
}

/**
 * Writes the editor's fields to a contact's vCard or Google person, or to a new contact when
 * `contactId` is absent. The engine queues the SyncbackContactTask.
 */
export function saveContact(
  { accountId, contactId, data }: { accountId: string; contactId?: string; data: object },
  engine: EngineEventsType = EngineEvents
): Promise<ContactDetailsJSON> {
  return contactId
    ? engine.request<ContactDetailsJSON>(Method.ContactsUpdate, {
        accountId,
        contactId,
        patch: data,
      })
    : engine.request<ContactDetailsJSON>(Method.ContactsCreate, { accountId, data });
}

export function importContacts(
  { accountId, vcf }: { accountId: string; vcf: string },
  engine: EngineEventsType = EngineEvents
): Promise<ContactsImportResult> {
  return engine.request<ContactsImportResult>(Method.ContactsImport, { accountId, vcf });
}

/** The contacts as one .vcf file, each account's asked for together. */
export async function exportContacts(
  contacts: Array<{ id: string; accountId: string }>,
  engine: EngineEventsType = EngineEvents
): Promise<string> {
  const byAccount = new Map<string, string[]>();
  for (const c of contacts) {
    byAccount.set(c.accountId, [...(byAccount.get(c.accountId) || []), c.id]);
  }
  const files: string[] = [];
  for (const [accountId, ids] of byAccount) {
    const { vcf } = await engine.request<{ vcf: string }>(Method.ContactsExport, {
      accountId,
      ids,
    });
    if (vcf) files.push(vcf);
  }
  return files.join('');
}
