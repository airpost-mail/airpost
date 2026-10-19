import React from 'react';
import { localized, Message, DraftEditingSession, useCapability } from 'mailspring-exports';
import { MetadataComposerToggleButton } from 'mailspring-component-kit';
import { PLUGIN_ID, PLUGIN_NAME, trackingUnavailableReason } from './link-tracking-constants';

type Props = { draft: Message; session: DraftEditingSession };

function errorMessage(error: Error) {
  return localized(
    `Unfortunately, link tracking servers are currently not available. Please try again later. Error: %@`,
    error.message
  );
}

const LinkTrackingButtonInner: React.FC<Props> = ({ draft, session }) => {
  // Subscribes to the store, so the memo'd button follows capabilities.changed.
  const { available } = useCapability('link-tracking');
  if (draft.plaintext) {
    return <span />;
  }
  return (
    <MetadataComposerToggleButton
      iconName="icon-composer-linktracking.png"
      pluginId={PLUGIN_ID}
      pluginName={PLUGIN_NAME}
      metadataEnabledValue={{ tracked: true }}
      errorMessage={errorMessage}
      draft={draft}
      session={session}
      unavailableReason={available ? undefined : trackingUnavailableReason()}
    />
  );
};
const LinkTrackingButton = React.memo(
  LinkTrackingButtonInner,
  (prev, next) =>
    prev.draft.metadataForPluginId(PLUGIN_ID) === next.draft.metadataForPluginId(PLUGIN_ID)
);
LinkTrackingButton.displayName = 'LinkTrackingButton';

export default LinkTrackingButton;
