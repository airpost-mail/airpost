import { Task } from './task';
import { AttributeValues } from '../models/model';
import * as Attributes from '../attributes';
import { localized } from '../../intl/localize';
import { ICSParticipantStatus } from '../calendar/ics';
import { SyncbackMetadataTask } from './syncback-metadata';
import DatabaseStore from '../database/store';
import { Message } from '../models/message';
import * as Actions from '../actions';

/**
 * Emails an iTIP message to a meeting organizer (RFC 5546): a REPLY with our status, or a
 * COUNTER proposing another time. The sync engine reads an absent `method` as REPLY. The
 * engine builds them (events.rsvp); the class inflates the tasks its deltas carry.
 */
export class EventRSVPTask extends Task {
  declare ics: string;
  declare icsRSVPStatus: ICSParticipantStatus;
  declare subject: string;
  declare messageId: string;
  organizerEmail?: string;
  declare method: 'REPLY' | 'COUNTER';
  declare comment: string;

  static attributes = {
    ...Task.attributes,

    ics: Attributes.String({
      modelKey: 'ics',
    }),
    icsRSVPStatus: Attributes.String({
      modelKey: 'icsRSVPStatus',
    }),
    to: Attributes.String({
      modelKey: 'to',
    }),
    subject: Attributes.String({
      modelKey: 'subject',
    }),
    messageId: Attributes.String({
      modelKey: 'messageId',
    }),
    method: Attributes.String({
      modelKey: 'method',
    }),
    comment: Attributes.String({
      modelKey: 'comment',
    }),
  };

  constructor(data: AttributeValues<typeof EventRSVPTask.attributes> = {}) {
    super(data);
  }

  /** Deprecated since plugin API 1.37.0 and removed in 2.0.0: use `Engine.request("events.rsvp")`. */
  static forReplying(_options: {
    to: string;
    accountId: string;
    messageId?: string;
    icsOriginalData: string;
    icsRSVPStatus: ICSParticipantStatus;
  }): EventRSVPTask {
    throw new Error('EventRSVPTask.forReplying is gone: use Engine.request("events.rsvp")');
  }

  /** Sends a COUNTER built elsewhere; events.rsvp with `counter` builds and queues its own. */
  static forProposingNewTime({
    accountId,
    to,
    messageId,
    ics,
    summary,
    comment,
  }: {
    accountId: string;
    to: string;
    messageId?: string;
    ics: string;
    summary: string;
    comment?: string;
  }) {
    return new EventRSVPTask({
      to,
      subject: localized('New time proposed: %@', summary),
      accountId,
      messageId,
      ics,
      method: 'COUNTER',
      comment,
    });
  }

  label() {
    return this.method === 'COUNTER'
      ? localized('Proposing a new time')
      : localized('Sending RSVP');
  }

  async onSuccess() {
    // A counter-proposal is not an answer, so the RSVP buttons must not show one.
    if (this.messageId && this.icsRSVPStatus && this.method !== 'COUNTER') {
      const msg = await DatabaseStore.find<Message>(Message, this.messageId);
      if (msg) {
        Actions.queueTask(
          SyncbackMetadataTask.forSaving({
            model: msg,
            pluginId: 'event-rsvp',
            value: {
              status: this.icsRSVPStatus,
              time: Date.now(),
            },
          })
        );
      }
    }

    // Pull the provider's latest calendar state after any RSVP response. Calendar
    // views observe the local Event table and update as soon as this sync lands.
    AppEnv.engineBridge.sendSyncCalendarNow(this.accountId);
  }
}
