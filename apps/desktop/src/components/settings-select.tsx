// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './kit/select';

export type SettingsSelectOption = [value: string, label: React.ReactNode];

// Radix Select reserves the empty string for "cleared", so an option whose value is '' goes
// through this sentinel.
const EMPTY_OPTION = '__empty__';
const toKey = (value: string) => (value === '' ? EMPTY_OPTION : value);
const fromKey = (key: string) => (key === EMPTY_OPTION ? '' : key);

/** A select of string options on the kit's Select, for settings and property rows. An option
 * may have the value ''. `id` and `aria-label` go on the trigger, so a `<label htmlFor>` points
 * at it. */
export function SettingsSelect(props: {
  id?: string;
  value: string;
  options: SettingsSelectOption[];
  onChange: (value: string) => void;
  'aria-label'?: string;
  placeholder?: string;
  disabled?: boolean;
  size?: 'sm' | 'default';
  className?: string;
}) {
  return (
    <Select
      value={toKey(props.value ?? '')}
      disabled={props.disabled}
      onValueChange={(key) => props.onChange(fromKey(key))}
    >
      <SelectTrigger
        id={props.id}
        size={props.size}
        className={props.className}
        aria-label={props['aria-label']}
      >
        <SelectValue placeholder={props.placeholder} />
      </SelectTrigger>
      <SelectContent>
        {props.options.map(([value, label]) => (
          <SelectItem key={toKey(value)} value={toKey(value)}>
            {label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
SettingsSelect.displayName = 'SettingsSelect';
