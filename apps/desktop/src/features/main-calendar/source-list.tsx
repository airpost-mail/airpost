import _ from 'underscore';
import React from 'react';

import { calcColor } from './helpers';
import { Calendar, Account } from 'mailspring-exports';
import { Checkbox } from '../../components/kit/checkbox';

const DISABLED_CALENDARS = 'core.calendar.disabledCalendars';

function renderCalendarToggles(calendars: Calendar[], disabledCalendars: string[]) {
  return calendars.map((calendar) => {
    const calendarId = calendar.id;
    const onToggle = () => {
      const cals = AppEnv.config.get(DISABLED_CALENDARS) || [];
      if (cals.includes(calendarId)) {
        cals.splice(cals.indexOf(calendarId), 1);
      } else {
        cals.push(calendarId);
      }
      AppEnv.config.set(DISABLED_CALENDARS, cals);
    };

    const checked = !disabledCalendars.includes(calendar.id);
    // The checked box fills with the calendar's colour.
    const style = { '--primary': calcColor(calendar.id) } as React.CSSProperties;
    // The whole row is the label, so a click anywhere on it toggles the calendar.
    return (
      <label title={calendar.name} className="toggle-wrap" key={`check-${calendar.id}`}>
        <Checkbox
          className="colored-checkbox"
          checked={checked}
          onCheckedChange={onToggle}
          style={style}
        />
        <span className="calendar-name">{calendar.name}</span>
      </label>
    );
  });
}

interface CalendarSourceListProps {
  accounts: Account[];
  calendars: Calendar[];
  disabledCalendars: string[];
}
export function CalendarSourceList(props: CalendarSourceListProps) {
  const calsByAccountId = _.groupBy(props.calendars, 'accountId');
  const accountSections = [];
  for (const accountId of Object.keys(calsByAccountId)) {
    const calendars = calsByAccountId[accountId];
    const account = props.accounts.find((a) => a.id === accountId);
    if (!account || !calendars || calendars.length === 0) {
      continue;
    }
    accountSections.push(
      <div key={accountId} className="account-calendars-wrap">
        <div className="account-label">{account.label}</div>
        {renderCalendarToggles(calendars, props.disabledCalendars)}
      </div>
    );
  }
  return <div className="calendar-source-list-wrap">{accountSections}</div>;
}
