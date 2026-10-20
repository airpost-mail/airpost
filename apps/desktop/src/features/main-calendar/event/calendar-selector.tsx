import React from 'react';
import { Calendar, Account, localized } from 'mailspring-exports';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '../../../components/kit/select';
import { calcColor, getEditableCalendars } from '../helpers';

interface CalendarSelectorProps {
  calendars: Calendar[];
  accounts: Account[];
  disabledCalendars: string[];
  selectedCalendarId: string;
  onChange: (calendarId: string, accountId: string) => void;
}

export function CalendarSelector({
  calendars,
  accounts,
  disabledCalendars,
  selectedCalendarId,
  onChange,
}: CalendarSelectorProps) {
  const editableCalendars = getEditableCalendars(calendars, disabledCalendars);

  // Group by account
  const calsByAccountId: Record<string, Calendar[]> = {};
  for (const cal of editableCalendars) {
    calsByAccountId[cal.accountId] = calsByAccountId[cal.accountId] || [];
    calsByAccountId[cal.accountId].push(cal);
  }

  const selectedColor = calcColor(selectedCalendarId);

  const handleChange = (calendarId: string) => {
    const calendar = calendars.find((c) => c.id === calendarId);
    if (calendar) {
      onChange(calendar.id, calendar.accountId);
    }
  };

  return (
    <div className="calendar-selector">
      <div className="calendar-selector-dot" style={{ backgroundColor: selectedColor }} />
      <Select value={selectedCalendarId || ''} onValueChange={handleChange}>
        <SelectTrigger size="sm" aria-label={localized('Calendar')}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {Object.keys(calsByAccountId).map((accountId) => {
            const cals = calsByAccountId[accountId];
            const account = accounts.find((a) => a.id === accountId);
            const accountLabel = account ? account.label : accountId;

            return (
              <SelectGroup key={accountId}>
                <SelectLabel>{accountLabel}</SelectLabel>
                {cals.map((cal) => (
                  <SelectItem key={cal.id} value={cal.id}>
                    {cal.name}
                  </SelectItem>
                ))}
              </SelectGroup>
            );
          })}
        </SelectContent>
      </Select>
    </div>
  );
}
CalendarSelector.displayName = 'CalendarSelector';
