import React, { useState } from 'react';
import { localized } from '../../../intl/localize';
import { Button } from '../../../components/kit/button';
import { Popover, PopoverContent, PopoverTrigger } from '../../../components/kit/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../components/kit/tooltip';

interface CalendarColorPickerProps {
  color: string; // Current color hex value (e.g., "#d93f35")
  onChange: (color: string) => void; // Callback when color is selected
  colors?: string[]; // Optional custom color palette
}

const DEFAULT_COLORS = [
  '#d93f35', // red
  '#e57c22', // orange
  '#f4c030', // yellow
  '#5eb25e', // green
  '#419bf9', // blue (matches @accent-primary)
  '#7b5db8', // purple
  '#9e9e9e', // gray
  '#795548', // brown
];

export const CalendarColorPicker: React.FC<CalendarColorPickerProps> = ({
  color,
  onChange,
  colors,
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const availableColors = colors || DEFAULT_COLORS;

  const handleColorSelect = (selectedColor: string) => {
    onChange(selectedColor);
    setIsOpen(false);
  };

  return (
    <div className="calendar-color-picker">
      <Popover open={isOpen} onOpenChange={setIsOpen}>
        <PopoverTrigger asChild>
          {/* The triggers' own data-slot would replace the buttons'. */}
          <Button
            variant="ghost"
            data-slot="button"
            className="color-picker-trigger"
            aria-label={localized('Color')}
          >
            <span className="color-dot" style={{ backgroundColor: color }} />
            <span className="color-picker-chevron" aria-hidden="true">
              ▾
            </span>
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="color-picker-dropdown">
          {availableColors.map((c) => (
            <Tooltip key={c}>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  data-slot="button"
                  aria-label={c}
                  aria-pressed={c === color}
                  className={`color-option ${c === color ? 'selected' : ''}`}
                  style={{ backgroundColor: c }}
                  onClick={() => handleColorSelect(c)}
                />
              </TooltipTrigger>
              <TooltipContent>{c}</TooltipContent>
            </Tooltip>
          ))}
        </PopoverContent>
      </Popover>
    </div>
  );
};
