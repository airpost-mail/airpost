import React from 'react';
import { localized } from 'mailspring-exports';

// Sharing publishes the report as a static page, which needs a server to host it. There is none
// yet, so the button is shown disabled.
export default function ShareButton() {
  const reason = localized('Sharing reports needs a sharing server, and none is configured.');
  return (
    <div className="btn btn-disabled" title={reason} aria-label={reason} aria-disabled="true">
      {localized('Share this Report')}
    </div>
  );
}
