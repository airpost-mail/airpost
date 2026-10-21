import moment, { Moment } from 'moment';
import { formatShortTime, momentZone } from '../../../intl/date-format';
import React from 'react';
import * as Actions from '../../../data/actions';
import { Calendar } from '../../../data/models/calendar';
import { Account } from '../../../data/models/account';
import DatabaseStore from '../../../data/database/store';
import DateUtils from '../../../utils/date';
import { Event } from '../../../data/models/event';
import { localized } from '../../../intl/localize';
import { Autolink } from '../../../mail/body/autolinker';
import * as CalendarDateUtils from '../../../data/calendar/calendar-date';
import * as Utils from '../../../data/models/utils';
import { DatePicker } from '../../../components/date-picker';
import { ScrollRegion } from '../../../components/scroll-region';
import { TabGroupRegion } from '../../../components/tab-group-region';
import TimePicker from '../../../components/time-picker';
import { Button } from '../../../components/kit/button';
import { IconButton } from '../../../components/kit/icon-button';
import { TextField } from '../../../components/kit/text-field';
import { Textarea } from '../../../components/kit/textarea';
import { EventAttendeesInput } from './attendees-input';
import {
  EventOccurrence,
  EventAttendee,
  occurrenceStartUnix,
  occurrenceEndUnix,
} from '../data-source';
import { canRespondToEvent, openProposeNewTimePopover } from './rsvp';
import { EventPropertyRow } from './property-row';
import {
  createCalendarEvent,
  inclusiveAllDayEnd,
  shiftEndWithStart,
  clampEnd,
  NewEventDraft,
} from '../helpers';
// CalendarColorPicker import removed - disabled until custom event colors are fully supported
import { CalendarSelector } from './calendar-selector';
import { LocationVideoInput } from './location-video-input';
import { AllDayToggle } from './all-day-toggle';
import { RepeatSelector, RepeatOption } from './repeat-selector';
import { AlertSelector, AlertTiming } from './alert-selector';
import { ShowAsSelector, ShowAsOption, defaultShowAs } from './show-as-selector';
import { EventPopoverActions } from './popover-actions';
import { TimeZoneSelector } from './timezone-selector';
import { parseEventIdFromOccurrence } from '../drag-utils';
import { showRecurringEventDialog } from './recurring-dialog';
import { editEvent } from '../../../engine/client/pim';
import type { EventPatch } from '../../../engine/protocol';

/**
 * Convert a RepeatOption UI value to an RRULE string (or null for 'none').
 */
function repeatOptionToRRule(option: RepeatOption): string | null {
  switch (option) {
    case 'daily':
      return 'FREQ=DAILY';
    case 'weekly':
      return 'FREQ=WEEKLY';
    case 'monthly':
      return 'FREQ=MONTHLY';
    case 'yearly':
      return 'FREQ=YEARLY';
    default:
      return null;
  }
}

/**
 * Convert an ICS recurrence frequency string to a RepeatOption UI value.
 */
function frequencyToRepeatOption(frequency: string | undefined): RepeatOption {
  switch (frequency?.toUpperCase()) {
    case 'DAILY':
      return 'daily';
    case 'WEEKLY':
      return 'weekly';
    case 'MONTHLY':
      return 'monthly';
    case 'YEARLY':
      return 'yearly';
    default:
      return 'none';
  }
}

interface CalendarEventPopoverProps {
  event: EventOccurrence;
  /** When true, the popover opens in edit mode to create a new event */
  isNewEvent?: boolean;
  /** Open straight into the editor rather than the read-only card. */
  startEditing?: boolean;
  /** Available calendars (required when isNewEvent is true) */
  calendars?: Calendar[];
  /** Available accounts (required when isNewEvent is true) */
  accounts?: Account[];
  /** Disabled calendar IDs (required when isNewEvent is true) */
  disabledCalendars?: string[];
  /** Whether the calendar containing this event is read-only */
  isCalendarReadOnly?: boolean;
  /** Receives the event as the editor would now save it, whenever a field the grid draws changes. */
  onDraftChange?: (draft: NewEventDraft) => void;
  onClosed?: () => void;
}

interface CalendarEventPopoverState {
  description: string;
  start: number;
  end: number;
  location: string;
  attendees: EventAttendee[];
  editing: boolean;
  title: string;
  // New fields for enhanced editing
  allDay: boolean;
  repeat: RepeatOption;
  /** The repeat value the event arrived with, so a save can tell whether the user changed it. */
  originalRepeat: RepeatOption;
  /** The zone the event arrived with; a series is rezoned only when the user picked another. */
  originalTimezone: string;
  alert: AlertTiming;
  showAs: ShowAsOption;
  /** Whether the user picked Show As; until then a new event's follows the all-day switch. */
  showAsChosen: boolean;
  /** The Show As value the event arrived with, so a save writes TRANSP only when it changed. */
  originalShowAs: ShowAsOption;
  calendarColor: string;
  timezone: string;
  showInvitees: boolean;
  showNotes: boolean;
  // Calendar selection for new events
  selectedCalendarId: string;
  selectedAccountId: string;
}

export class CalendarEventPopover extends React.Component<
  CalendarEventPopoverProps,
  CalendarEventPopoverState
> {
  private attendeesInputRef = React.createRef<EventAttendeesInput>();
  private notesTextareaRef = React.createRef<HTMLTextAreaElement>();

  constructor(props: CalendarEventPopoverProps) {
    super(props);
    const { description, location, attendees, title, isAllDay } = this.props.event;
    // The popover edits in instants; seed all-day events from their derived day boundaries.
    const start = occurrenceStartUnix(this.props.event);
    const end = occurrenceEndUnix(this.props.event);

    this.state = {
      description,
      start,
      end,
      location,
      title,
      editing: !!this.props.isNewEvent || !!this.props.startEditing,
      attendees,
      // Initialize new fields with defaults
      allDay: isAllDay || false,
      repeat: 'none',
      originalRepeat: 'none',
      originalTimezone: DateUtils.timeZone,
      alert: '10min',
      showAs: this._eventShowAs(),
      showAsChosen: false,
      originalShowAs: this._eventShowAs(),
      calendarColor: '#419bf9',
      timezone: DateUtils.timeZone,
      showInvitees: attendees && attendees.length > 0,
      showNotes: !!description,
      selectedCalendarId: this.props.event.calendarId,
      selectedAccountId: this.props.event.accountId,
    };
  }

  componentDidUpdate(prevProps: CalendarEventPopoverProps, prevState: CalendarEventPopoverState) {
    // Update state when event prop changes
    if (prevProps.event !== this.props.event) {
      const { description, location, attendees, title } = this.props.event;
      const start = occurrenceStartUnix(this.props.event);
      const end = occurrenceEndUnix(this.props.event);
      const showAs = this._eventShowAs();
      this.setState({
        description,
        start,
        end,
        location,
        attendees,
        title,
        showAs,
        showAsChosen: false,
        originalShowAs: showAs,
      });
    }

    const draft = this._draft();
    if (!Utils.isEqualReact(draft, this._draft(prevState))) {
      this.props.onDraftChange?.(draft);
    }

    // Autofocus invitees input when section is expanded
    // Use requestAnimationFrame to ensure DOM is ready
    if (this.state.showInvitees && !prevState.showInvitees) {
      requestAnimationFrame(() => {
        if (this.attendeesInputRef.current) {
          this.attendeesInputRef.current.focus();
        }
      });
    }
    // Autofocus notes textarea when section is expanded
    if (this.state.showNotes && !prevState.showNotes) {
      requestAnimationFrame(() => {
        if (this.notesTextareaRef.current) {
          this.notesTextareaRef.current.focus();
        }
      });
    }
  }

  // The Repeat control defaults to 'none' until this has read the event; the save writes the
  // rule only when the control was changed, so a save before then leaves the rule alone.
  async _loadEditDefaults(): Promise<void> {
    const { rrule, timezone: eventTz } = this.props.event;
    const frequency = rrule ? /(?:^|;)FREQ=([^;]+)/i.exec(rrule)?.[1] : undefined;
    const repeat = frequencyToRepeatOption(frequency);
    const timezone = eventTz || this.state.timezone;
    this.setState({ repeat, timezone, originalRepeat: repeat, originalTimezone: timezone });
  }

  onEdit = async () => {
    await this._loadEditDefaults();
    this.setState({ editing: true });
  };

  componentDidMount() {
    if (this.props.startEditing && !this.props.isNewEvent) {
      this._loadEditDefaults();
    }
  }

  componentWillUnmount() {
    this.props.onClosed?.();
  }

  _draft(state: CalendarEventPopoverState = this.state): NewEventDraft {
    const { title, start, end, allDay, selectedCalendarId, selectedAccountId } = state;
    return {
      title,
      start,
      end,
      isAllDay: allDay,
      calendarId: selectedCalendarId,
      accountId: selectedAccountId,
    };
  }

  _eventShowAs(): ShowAsOption {
    if (this.props.isNewEvent) return defaultShowAs(this.props.event.isAllDay);
    return this.props.event.isFree ? 'TRANSPARENT' : 'OPAQUE';
  }

  updateAllDay = (allDay: boolean): void => {
    const followsAllDay = this.props.isNewEvent && !this.state.showAsChosen;
    const showAs = followsAllDay ? defaultShowAs(allDay) : this.state.showAs;
    this.setState({ allDay, showAs });
  };

  updateShowAs = (showAs: ShowAsOption): void => {
    this.setState({ showAs, showAsChosen: true });
  };

  /** The Show As value to write, or undefined when the user left it as the event had it. */
  _changedShowAs(): ShowAsOption | undefined {
    return this.state.showAs !== this.state.originalShowAs ? this.state.showAs : undefined;
  }

  getStartMoment = () => moment(this.state.start * 1000);
  getEndMoment = () => moment(this.state.end * 1000);

  /** The popover's title, location, notes and guests, as an events.update patch. */
  _propertyPatch(): EventPatch {
    return {
      summary: this.state.title || localized('New Event'),
      location: this.state.location || '',
      description: this.state.description || '',
      attendees: (this.state.attendees || []).map((a) => ({
        email: a.email,
        name: a.name || undefined,
      })),
    };
  }

  saveEdits = async (): Promise<void> => {
    try {
      if (this.props.isNewEvent) {
        await this._createNewEvent();
        return;
      }

      // Extract the real event ID from the occurrence ID (format: `${eventId}-e${idx}`)
      const eventId = parseEventIdFromOccurrence(this.props.event.id);

      // Fetch the actual Event from the database
      const event = await DatabaseStore.find<Event>(Event, eventId);
      if (!event) {
        console.error(`Could not find event with id ${eventId} to update`);
        this.setState({ editing: false });
        return;
      }

      const isRecurring = this.props.event.isRecurring && !event.isRecurrenceException();

      if (this.props.event.isException) {
        // This occurrence already has an exception — always edit the exception directly.
        // The user already chose "this occurrence only" when the exception was created;
        // asking again would be confusing and risks creating duplicate exception VEVENTs.
        await this._saveOccurrenceException(event);
      } else if (isRecurring) {
        const choice = await showRecurringEventDialog('edit', this.props.event.title);
        if (choice === 'cancel') {
          return;
        }
        if (choice === 'this-occurrence') {
          await this._saveOccurrenceException(event);
        } else {
          await this._saveAllOccurrences(event, true);
        }
      } else {
        await this._saveAllOccurrences(event, false);
      }

      this.setState({ editing: false });
      Actions.closePopover();
    } catch (error) {
      // Stay in edit mode so a failed save doesn't discard what the user typed
      console.error('Failed to save event edits:', error);
      AppEnv.showErrorDialog({
        title: localized('Update Failed'),
        message: localized('Failed to update the event. Please try again.'),
      });
    }
  };

  /**
   * Save edits to the master event (used for non-recurring events and "all occurrences").
   *
   * A series moves by the change made to this occurrence, so the occurrences before it stay,
   * and its overrides keep the times the user gave them (the engine shifts their RECURRENCE-IDs
   * with the series).
   */
  async _saveAllOccurrences(event: Event, isRecurring: boolean): Promise<void> {
    const patch: EventPatch = {
      ...this._propertyPatch(),
      start: this.state.start,
      end: this.state.end,
      isAllDay: this.state.allDay,
    };
    // Passing the picker's zone unchanged would rewrite every EXDATE and RECURRENCE-ID of a
    // UTC series into the machine's zone, so a series gets only a zone the user picked.
    if (!isRecurring || this.state.timezone !== this.state.originalTimezone) {
      patch.timezone = this.state.timezone;
    }
    // The Repeat control can't express INTERVAL, BYDAY, COUNT, UNTIL or RDATE, so writing it back
    // unchanged would flatten the rule and bump SEQUENCE for every guest.
    if (this.state.repeat !== this.state.originalRepeat) {
      patch.recurrenceRule = repeatOptionToRRule(this.state.repeat);
    }
    const showAs = this._changedShowAs();
    if (showAs) {
      patch.transparency = showAs;
    }
    await editEvent({
      event,
      scope: 'all',
      occurrenceStart: isRecurring ? occurrenceStartUnix(this.props.event) : undefined,
      patch,
      description: localized('Edit event'),
    });
  }

  /**
   * Saves the edits on this occurrence alone: an override in the series' resource, added, or
   * changed when the occurrence already has one (RFC 4791 section 4.1).
   */
  async _saveOccurrenceException(masterEvent: Event): Promise<void> {
    // The slot the occurrence fills: for one already moved, its RECURRENCE-ID, not its start.
    const occurrenceStart =
      this.props.event.recurrenceIdStart ?? occurrenceStartUnix(this.props.event);
    await editEvent({
      event: masterEvent,
      scope: 'this',
      occurrenceStart,
      patch: {
        ...this._propertyPatch(),
        start: this.state.start,
        end: this.state.end,
        isAllDay: this.state.allDay,
        transparency: this._changedShowAs(),
      },
      description: localized('Edit occurrence'),
    });
  }

  _createNewEvent = async (): Promise<void> => {
    const {
      title,
      start,
      end,
      allDay,
      location,
      description,
      attendees,
      repeat,
      timezone,
      showAs,
      selectedCalendarId,
      selectedAccountId,
    } = this.state;

    Actions.closePopover();

    await createCalendarEvent({
      summary: title || localized('New Event'),
      start: new Date(start * 1000),
      end: new Date(end * 1000),
      isAllDay: allDay,
      calendarId: selectedCalendarId,
      accountId: selectedAccountId,
      description: description || undefined,
      location: location || undefined,
      attendees:
        attendees && attendees.length > 0
          ? attendees.map((a) => ({ email: a.email, name: a.name ?? undefined }))
          : undefined,
      recurrenceRule: repeatOptionToRRule(repeat) || undefined,
      timezone,
      transparency: showAs,
    });
  };

  // If on the hour, formats as "3 PM", else formats as "3:15 PM"

  updateAttendees = (attendees: EventAttendee[]): void => {
    this.setState({ attendees });
  };

  updateField = <K extends keyof CalendarEventPopoverState>(
    key: K,
    value: CalendarEventPopoverState[K]
  ): void => {
    this.setState({ [key]: value } as Pick<CalendarEventPopoverState, K>);
  };

  updateStart = (start: number): void => {
    const { start: oldStart, end, allDay } = this.state;
    this.setState({ start, end: shiftEndWithStart(oldStart, end, start, allDay) });
  };

  updateEnd = (end: number): void => {
    this.setState({ end: clampEnd(this.state.start, end, this.state.allDay) });
  };

  updateCalendar = (calendarId: string, accountId: string): void => {
    this.setState({ selectedCalendarId: calendarId, selectedAccountId: accountId });
  };

  renderEditable = () => {
    const {
      title,
      description,
      start,
      end,
      location,
      attendees,
      allDay,
      repeat,
      alert,
      showAs,
      timezone,
      showInvitees,
      showNotes,
    } = this.state;

    const notes = extractNotesFromDescription(description);

    return (
      <div className="calendar-event-popover editing" tabIndex={0}>
        <TabGroupRegion className="popover-form">
          {/* Title row */}
          <div className="title-wrapper">
            <TextField
              className="title"
              aria-label={localized('Event title')}
              placeholder={localized('New Event')}
              value={title}
              onChange={(e) => this.updateField('title', e.target.value)}
            />
            {/* CalendarColorPicker disabled until custom event colors are fully supported */}
          </div>

          {/* Everything between the title and the buttons scrolls; the buttons stay put. */}
          <div className="popover-body">
            {/* Calendar selector - only shown for new events */}
            {this.props.isNewEvent && this.props.calendars && this.props.accounts && (
              <CalendarSelector
                calendars={this.props.calendars}
                accounts={this.props.accounts}
                disabledCalendars={this.props.disabledCalendars || []}
                selectedCalendarId={this.state.selectedCalendarId}
                onChange={this.updateCalendar}
              />
            )}

            {/* Location with video call toggle */}
            <LocationVideoInput
              value={location}
              onChange={(value) => this.updateField('location', value)}
              onVideoToggle={() => {
                // Placeholder: could add video call link
              }}
            />

            {/* All-day toggle */}
            <AllDayToggle checked={allDay} onChange={this.updateAllDay} />

            {/* Start/End times using property rows */}
            <EventPropertyRow label={localized('starts:')}>
              <DatePicker value={start * 1000} onChange={(ts) => this.updateStart(ts / 1000)} />
              {!allDay && (
                <TimePicker value={start * 1000} onChange={(ts) => this.updateStart(ts / 1000)} />
              )}
            </EventPropertyRow>

            <EventPropertyRow label={localized('ends:')}>
              <DatePicker
                value={(allDay ? inclusiveAllDayEnd(end) : end) * 1000}
                onChange={(ts) =>
                  this.updateEnd(
                    allDay
                      ? CalendarDateUtils.nextDayStartUnix(
                          CalendarDateUtils.calendarDateFromUnix(ts / 1000)
                        )
                      : ts / 1000
                  )
                }
              />
              {!allDay && (
                <TimePicker value={end * 1000} onChange={(ts) => this.updateEnd(ts / 1000)} />
              )}
            </EventPropertyRow>

            {/* Time zone selector */}
            <TimeZoneSelector
              value={timezone}
              onChange={(value) => this.updateField('timezone', value)}
            />

            {/* Repeat selector */}
            <RepeatSelector
              value={repeat}
              onChange={(value) => this.updateField('repeat', value)}
            />

            {/* Alert selector */}
            <AlertSelector value={alert} onChange={(value) => this.updateField('alert', value)} />

            {/* Show as selector */}
            <ShowAsSelector value={showAs} onChange={this.updateShowAs} />

            {/* Invitees section - collapsible */}
            {showInvitees ? (
              <div className="expanded-section">
                <div className="section-header">
                  <span className="section-title">{localized('Invitees')}</span>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="section-close"
                    aria-label={localized('Hide Invitees')}
                    onClick={() => this.setState({ showInvitees: false })}
                  >
                    ×
                  </Button>
                </div>
                <EventAttendeesInput
                  ref={this.attendeesInputRef}
                  className="event-participant-field"
                  attendees={attendees}
                  change={(val) => this.updateField('attendees', val)}
                />
              </div>
            ) : (
              <Button
                variant="ghost"
                className="action-link"
                onClick={() => this.setState({ showInvitees: true })}
              >
                {localized('Add Invitees')}
              </Button>
            )}

            {/* Notes section - collapsible */}
            {showNotes ? (
              <div className="expanded-section">
                <div className="section-header">
                  <span className="section-title">{localized('Notes')}</span>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="section-close"
                    aria-label={localized('Hide Notes')}
                    onClick={() => this.setState({ showNotes: false })}
                  >
                    ×
                  </Button>
                </div>
                <Textarea
                  ref={this.notesTextareaRef}
                  value={notes}
                  aria-label={localized('Notes')}
                  placeholder={localized('Add notes or URL...')}
                  onChange={(e) => this.updateField('description', e.target.value)}
                />
              </div>
            ) : (
              <Button
                variant="ghost"
                className="action-link"
                onClick={() => this.setState({ showNotes: true })}
              >
                {localized('Add Notes or URL')}
              </Button>
            )}
          </div>

          {/* Action buttons */}
          <EventPopoverActions onSave={this.saveEdits} onCancel={() => Actions.closePopover()} />
        </TabGroupRegion>
      </div>
    );
  };

  // A writable calendar and a meeting we organize: only the organizer revises one (RFC 5546
  // section 2.1.4), the same test canMoveEvent applies to dragging.
  _isEditable(): boolean {
    return !this.props.isCalendarReadOnly && (this.props.isNewEvent || this.props.event.isMine);
  }

  render() {
    if (this._isEditable() && (this.state.editing || this.props.isNewEvent)) {
      return this.renderEditable();
    }
    return (
      <CalendarEventPopoverUnenditable
        {...this.props}
        editable={this._isEditable()}
        onEdit={this.onEdit}
      />
    );
  }
}

class CalendarEventPopoverUnenditable extends React.Component<
  CalendarEventPopoverProps & { editable: boolean; onEdit: () => void }
> {
  descriptionRef = React.createRef<HTMLDivElement>();

  renderTime() {
    const { event } = this.props;

    if (event.isAllDay === true) {
      const startMoment = moment(CalendarDateUtils.dayStartUnix(event.startDate) * 1000);
      const date = startMoment.format('dddd, MMMM D'); // e.g. Tuesday, February 22
      const lastDay = moment(CalendarDateUtils.dayStartUnix(event.endDate) * 1000);
      return (
        <div>
          {event.startDate === event.endDate ? date : `${date} – ${lastDay.format('MMMM D')}`}
          <br />
          {localized('All day')}
        </div>
      );
    }

    const startMoment = moment(event.start * 1000);
    const date = startMoment.format('dddd, MMMM D'); // e.g. Tuesday, February 22
    const endMoment = moment(event.end * 1000);
    const timeRange = `${formatTime(startMoment)} - ${formatTime(endMoment)}`;
    return (
      <div>
        {date}
        <br />
        {timeRange}
      </div>
    );
  }

  componentDidMount() {
    this.autolink();
  }

  componentDidUpdate() {
    this.autolink();
  }

  autolink() {
    if (!this.descriptionRef.current) return;
    Autolink(this.descriptionRef.current, {
      async: false,
      telAggressiveMatch: true,
    });
  }

  // The card is what a double-click on a meeting we cannot edit produces, so it carries the
  // one action that replaces editing.
  _renderProposeNewTime() {
    const { event } = this.props;
    if (!canRespondToEvent(event)) {
      return null;
    }
    return (
      <div className="section propose-time-action">
        <Button
          variant="link"
          onClick={() => {
            Actions.closePopover();
            openProposeNewTimePopover(event);
          }}
        >
          {localized('Propose a new time') + '...'}
        </Button>
      </div>
    );
  }

  render() {
    const { event, onEdit, editable } = this.props;
    const { title, description, location, attendees } = event;

    const notes = extractNotesFromDescription(description);

    return (
      <div className="calendar-event-popover" tabIndex={0}>
        <div className="title-wrapper">
          <div className="title">{title}</div>
          {editable && (
            <IconButton
              className="edit-icon"
              icon="edit-icon.png"
              label={localized('Edit')}
              onClick={onEdit}
            />
          )}
        </div>
        {/* Everything under the title scrolls as one body when the card is taller than the window. */}
        <div className="popover-body">
          {location && (
            <div className="location">
              {location.startsWith('http') || location.startsWith('tel:') ? (
                <a href={location}>{location}</a>
              ) : (
                location
              )}
            </div>
          )}
          <div className="section">{this.renderTime()}</div>
          {this._renderProposeNewTime()}
          <ScrollRegion className="section invitees">
            <div className="label">{localized(`Invitees`)}: </div>
            <div className="invitees-list">
              {sortAttendeesByStatus(attendees).map((a, idx) => {
                const partstat = a.partstat || 'NEEDS-ACTION';
                const questionMarkIcon = (
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                    <path
                      d="M3.5 3.5a1.5 1.5 0 0 1 2.6 1c0 1-1.1 1-1.1 2"
                      stroke="currentColor"
                      strokeWidth="1.2"
                      strokeLinecap="round"
                    />
                    <circle cx="5" cy="8.5" r="0.75" fill="currentColor" />
                  </svg>
                );
                let statusIcon: React.ReactNode;
                let statusClass = 'needs-action';
                if (partstat === 'ACCEPTED') {
                  statusIcon = (
                    <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                      <path
                        d="M1.5 5.5L4 8l4.5-6"
                        stroke="currentColor"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  );
                  statusClass = 'accepted';
                } else if (partstat === 'DECLINED') {
                  statusIcon = (
                    <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                      <path
                        d="M2 2l6 6M8 2l-6 6"
                        stroke="currentColor"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                      />
                    </svg>
                  );
                  statusClass = 'declined';
                } else if (partstat === 'TENTATIVE') {
                  statusIcon = questionMarkIcon;
                  statusClass = 'tentative';
                } else {
                  statusIcon = questionMarkIcon;
                }
                return (
                  <div key={idx} className={`attendee-chip ${statusClass}`}>
                    <span className="attendee-status">{statusIcon}</span>
                    <span className="attendee-name">{a.name || a.email}</span>
                  </div>
                );
              })}
            </div>
          </ScrollRegion>
          <ScrollRegion className="section description">
            <div className="description">
              <div className="label">{localized(`Notes`)}: </div>
              <div ref={this.descriptionRef}>{notes}</div>
            </div>
          </ScrollRegion>
        </div>
      </div>
    );
  }
}

function sortAttendeesByStatus(attendees: EventAttendee[]): EventAttendee[] {
  const statusOrder: { [partstat: string]: number } = {
    ACCEPTED: 0,
    TENTATIVE: 1,
    'NEEDS-ACTION': 1,
    DECLINED: 2,
  };
  return [...attendees].sort((a, b) => {
    const aOrder = statusOrder[a.partstat ?? ''] ?? 1;
    const bOrder = statusOrder[b.partstat ?? ''] ?? 1;
    if (aOrder !== bOrder) return aOrder - bOrder;
    const aName = (a.name || a.email).toLowerCase();
    const bName = (b.name || b.email).toLowerCase();
    return aName.localeCompare(bName);
  });
}

// A description is markup from whoever sent the invitation. DOMParser's document is inert, so the
// images it references are never fetched.
function extractNotesFromDescription(description: string) {
  const doc = new DOMParser().parseFromString(description, 'text/html');
  const descriptionRoot = doc.body;

  // The parsed document has no layout, so block boundaries become newlines here; Google writes
  // each line of a description as <br>-separated HTML.
  descriptionRoot.querySelectorAll('br').forEach((br) => br.replaceWith('\n'));
  descriptionRoot
    .querySelectorAll('p, div, li, tr, h1, h2, h3, h4, h5, h6, blockquote')
    .forEach((block) => block.append('\n'));

  // The parser moves <meta> tags that lead the description into <head>.
  const els = doc.querySelectorAll('meta[itemprop=description]');
  let notes: string;
  if (els.length) {
    notes = Array.from(els)
      .map((el) => (el as HTMLMetaElement).content)
      .join('\n');
  } else {
    // An element's textContent is never null.
    notes = descriptionRoot.textContent ?? '';
  }
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const nextNotes = notes.replace('\n\n', '\n');
    if (nextNotes === notes) {
      break;
    }
    notes = nextNotes;
  }
  return notes;
}

function formatTime(momentTime: Moment) {
  return formatShortTime(momentTime.toDate(), { timeZone: momentZone(momentTime as any) });
}
