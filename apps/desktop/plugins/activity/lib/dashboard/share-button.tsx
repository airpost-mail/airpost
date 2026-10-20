import React from 'react';
import { localized, CapabilityStore } from 'mailspring-exports';

// Sharing publishes the report as a static page on a Tern server. Until the capability store
// reports one, the button is shown disabled.
export default function ShareButton() {
  const reason = CapabilityStore.unavailableMessage('activity-sharing');
  return (
    <div className="btn btn-disabled" title={reason} aria-label={reason} aria-disabled="true">
      {localized('Share this Report')}
    </div>
  );
}
