import React from 'react';
import { localized } from '../../../intl/localize';
import { EventPropertyRow } from './property-row';
import { PropertySelect } from './property-select';

/** The event's TRANSP value, which is all iCalendar records about busy and free. */
export type ShowAsOption = 'OPAQUE' | 'TRANSPARENT';

/**
 * Show As for a new event: Free all day, Busy otherwise. Google Calendar, Apple Calendar, Outlook
 * and Thunderbird (calendar.events.defaultTransparency.*) all start events this way, so an
 * all-day reminder or birthday does not block the day in free/busy lookups.
 */
export function defaultShowAs(isAllDay: boolean): ShowAsOption {
  return isAllDay ? 'TRANSPARENT' : 'OPAQUE';
}

const SHOW_AS_OPTIONS: { value: ShowAsOption; label: string }[] = [
  { value: 'OPAQUE', label: localized('Busy') },
  { value: 'TRANSPARENT', label: localized('Free') },
];

interface ShowAsSelectorProps {
  value: ShowAsOption;
  onChange: (value: ShowAsOption) => void;
}

export const ShowAsSelector: React.FC<ShowAsSelectorProps> = ({ value, onChange }) => {
  return (
    <EventPropertyRow label={localized('show as:')}>
      <PropertySelect
        className="show-as-select"
        label={localized('Show as')}
        value={value}
        options={SHOW_AS_OPTIONS}
        onChange={onChange}
      />
    </EventPropertyRow>
  );
};
