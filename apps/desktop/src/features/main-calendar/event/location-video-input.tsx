import React from 'react';
import { localized } from 'mailspring-exports';
import { IconButton } from '../../../components/kit/icon-button';
import { TextField } from '../../../components/kit/text-field';

interface LocationVideoInputProps {
  value: string; // Current location value
  onChange: (value: string) => void; // Callback when location changes
  onVideoToggle?: () => void; // Optional callback for video button click
  showVideoButton?: boolean; // Whether to show video button (default: true)
}

export const LocationVideoInput: React.FC<LocationVideoInputProps> = ({
  value,
  onChange,
  onVideoToggle,
  showVideoButton = true,
}) => {
  const handleVideoClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (onVideoToggle) {
      onVideoToggle();
    }
  };

  return (
    <div className="location-video-input">
      <TextField
        className="location-input"
        aria-label={localized('Location or Video Call')}
        placeholder={localized('Add Location or Video Call')}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      {showVideoButton && (
        <IconButton
          className="video-toggle"
          icon="ic-eventcard-videocall.png"
          label={localized('Add Video Call')}
          onClick={handleVideoClick}
        />
      )}
    </div>
  );
};
