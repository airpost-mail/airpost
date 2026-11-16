import React from 'react';
import * as Actions from '../../../data/actions';
import { localized } from '../../../intl/localize';
import { Button } from '../../../components/kit/button';

export function CalendarEmptyState() {
  const onOpenAccountPreferences = () => {
    Actions.switchPreferencesTab('Accounts');
    Actions.openPreferences();
  };

  return (
    <div className="calendar-empty-state">
      <div className="calendar-empty-state-content">
        <h2 className="calendar-empty-state-title">{localized('No Calendars')}</h2>
        <p className="calendar-empty-state-message">
          {localized(
            'None of your connected accounts provide calendars. Airpost supports calendars from Gmail and other providers with CalDAV support.'
          )}
        </p>
        <Button size="lg" onClick={onOpenAccountPreferences}>
          {localized('Add a Calendar Account')}
        </Button>
      </div>
    </div>
  );
}
