// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { SettingsSelect } from '../../../components/settings-select';

interface PropertySelectProps<T extends string> {
  className: string;
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}

/** The kit Select an event property row offers its choices in. */
export function PropertySelect<T extends string>({
  className,
  label,
  value,
  options,
  onChange,
}: PropertySelectProps<T>) {
  return (
    <SettingsSelect
      size="sm"
      className={className}
      aria-label={label}
      value={value}
      options={options.map((opt) => [opt.value, opt.label])}
      onChange={(v) => onChange(v as T)}
    />
  );
}
