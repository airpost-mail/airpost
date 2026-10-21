import React from 'react';
import { localized } from '../../../intl/localize';
import { Button } from '../../../components/kit/button';

interface EventPopoverActionsProps {
  onSave: () => void;
  onCancel: () => void;
  saveDisabled?: boolean;
  saveLabel?: string;
}

export const EventPopoverActions: React.FC<EventPopoverActionsProps> = ({
  onSave,
  onCancel,
  saveDisabled = false,
  saveLabel,
}) => {
  return (
    <div className="event-popover-actions">
      <Button variant="outline" onClick={onCancel}>
        {localized('Cancel')}
      </Button>
      <Button onClick={onSave} disabled={saveDisabled}>
        {saveLabel || localized('Save')}
      </Button>
    </div>
  );
};
