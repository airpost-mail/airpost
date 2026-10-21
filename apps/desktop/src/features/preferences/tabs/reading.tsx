import React from 'react';
import { localized } from '../../../intl/localize';
import {
  SettingsPage,
  SettingsSection,
  ConfigSwitchRow,
  ConfigSelectRow,
  ConfigSegmentedRow,
} from '../../../components/settings';

export default function PreferencesReading() {
  return (
    <SettingsPage
      title={localized('Reading')}
      description={localized('How messages open and show.')}
    >
      <SettingsSection>
        <ConfigSelectRow
          keyPath="core.reading.markAsReadDelay"
          label={localized('Mark as read')}
          options={[
            [0, localized('Right away')],
            [500, localized('After half a second')],
            [2000, localized('After 2 seconds')],
            [-1, localized('Only when I say')],
          ]}
        />
        <ConfigSwitchRow
          keyPath="core.reading.autoloadImages"
          label={localized('Load images automatically')}
          description={localized('Images from senders can tell them you opened the message.')}
        />
        <ConfigSegmentedRow
          keyPath="core.reading.descendingOrderMessageList"
          label={localized('Conversation order')}
          options={[
            [false, localized('Oldest first')],
            [true, localized('Newest first')],
          ]}
        />
        <ConfigSwitchRow
          keyPath="core.reading.detailedNames"
          label={localized('Show full names of recipients')}
        />
        <ConfigSwitchRow
          keyPath="core.reading.detailedHeaders"
          label={localized('Show all message headers')}
        />
        <ConfigSwitchRow
          keyPath="core.reading.restrictMaxWidth"
          label={localized('Limit line length in messages')}
          description={localized('Easier to read in wide windows.')}
        />
      </SettingsSection>

      <SettingsSection title={localized('Folders and labels')}>
        <ConfigSwitchRow
          keyPath="core.workspace.showImportant"
          label={localized("Show Gmail's important markers")}
          description={localized('Gmail accounts only.')}
        />
        <ConfigSwitchRow
          keyPath="core.workspace.showUnreadForAllCategories"
          label={localized('Show unread counts on every folder')}
        />
      </SettingsSection>

      <SettingsSection title={localized('Swipe and delete')}>
        <ConfigSwitchRow
          keyPath="core.reading.swipeDisabled"
          invert
          label={localized('Swipe in the message list')}
        />
        <ConfigSegmentedRow
          keyPath="core.reading.backspaceDelete"
          label={localized('Swipe and the Delete key')}
          options={[
            [false, localized('Archive')],
            [true, localized('Move to Trash')],
          ]}
        />
      </SettingsSection>

      <SettingsSection title={localized('Contact photos')}>
        <ConfigSwitchRow
          keyPath="core.reading.loadGravatar"
          label={localized('Load photos from Gravatar')}
          description={localized(
            "Sends a scrambled form of each sender's address to gravatar.com."
          )}
        />
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesReading.displayName = 'PreferencesReading';
