import React from 'react';
import { localized, CapabilityStore } from 'mailspring-exports';
import { Button, Tooltip, TooltipContent, TooltipTrigger } from 'mailspring-component-kit';

// Sharing publishes the report as a static page on an Airpost server. Until the capability store
// reports one, the button is shown disabled. A disabled button gets no pointer events, so the
// tooltip with the reason hangs off a focusable wrapper.
export default function ShareButton() {
  const reason = CapabilityStore.unavailableMessage('activity-sharing');
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="share-button" tabIndex={0}>
          <Button variant="outline" disabled>
            {localized('Share this Report')}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>{reason}</TooltipContent>
    </Tooltip>
  );
}
