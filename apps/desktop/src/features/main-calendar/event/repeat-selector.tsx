import React from 'react';
import { localized } from 'mailspring-exports';
import { EventPropertyRow } from './property-row';
import { PropertySelect } from './property-select';

export type RepeatOption = 'none' | 'daily' | 'weekly' | 'monthly' | 'yearly';

interface RepeatSelectorProps {
  value: RepeatOption;
  onChange: (value: RepeatOption) => void;
}

const REPEAT_OPTIONS: { value: RepeatOption; label: string }[] = [
  { value: 'none', label: localized('None') },
  { value: 'daily', label: localized('Every Day') },
  { value: 'weekly', label: localized('Every Week') },
  { value: 'monthly', label: localized('Every Month') },
  { value: 'yearly', label: localized('Every Year') },
];

export const RepeatSelector: React.FC<RepeatSelectorProps> = ({ value, onChange }) => {
  return (
    <EventPropertyRow label={localized('repeat:')}>
      <PropertySelect
        className="repeat-select"
        label={localized('Repeat')}
        value={value}
        options={REPEAT_OPTIONS}
        onChange={onChange}
      />
    </EventPropertyRow>
  );
};
