import React from 'react';
import { AccountStore, Actions, localized } from 'mailspring-exports';
import {
  SettingsPage,
  SettingsSection,
  SettingsRow,
  StatusPill,
  ConfigSwitchRow,
  ConfigSelectRow,
  ConfigSegmentedRow,
} from 'mailspring-component-kit';
import { Button } from '../../../components/kit/button';

function onTurnWarningsBackOn() {
  localStorage.removeItem('recipientWarningBlacklist');
}

export default function PreferencesComposing() {
  const grammar = !!AppEnv.config.get('core.composing.grammarCheck');
  const accounts = AccountStore.accounts();
  const spellLanguageApplies = (
    AppEnv.config.getSchema('core.composing.spellcheckDefaultLanguage') as any
  ).platforms?.includes(process.platform);
  return (
    <SettingsPage
      title={localized('Composing & sending')}
      description={localized('Writing, replying and what happens when you click Send.')}
    >
      <SettingsSection title={localized('Writing')}>
        <ConfigSegmentedRow
          keyPath="core.composing.html"
          label={localized('Formatting')}
          description={localized(
            'Plain text turns off links, images and most other features. To write one plain-text message, hold Alt or Option when you click Compose or Reply.'
          )}
          options={[
            [true, localized('Rich text')],
            [false, localized('Plain text')],
          ]}
        />
        <ConfigSwitchRow keyPath="core.composing.spellcheck" label={localized('Check spelling')} />
        {spellLanguageApplies && (
          <ConfigSelectRow
            keyPath="core.composing.spellcheckDefaultLanguage"
            label={localized('Spelling language')}
          />
        )}
        <SettingsRow
          label={localized('Check grammar')}
          description={
            <>
              {localized('Uses LanguageTool.')}{' '}
              <a onClick={() => Actions.switchPreferencesTab('Integrations')}>
                {localized('Set up in Integrations')}
              </a>
            </>
          }
        >
          <StatusPill kind={grammar ? 'ok' : 'off'}>
            {grammar ? localized('On') : localized('Off')}
          </StatusPill>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title={localized('Sending')}>
        <ConfigSelectRow
          keyPath="core.sending.undoSend"
          label={localized('Undo Send')}
          description={localized('How long you have to take a message back.')}
          options={[
            [5000, localized('5 seconds')],
            [15000, localized('15 seconds')],
            [30000, localized('30 seconds')],
            [60000, localized('60 seconds')],
            [0, localized('Off')],
          ]}
        />
        <ConfigSegmentedRow
          keyPath="core.sending.defaultSendType"
          label={localized('Send button')}
          options={[
            ['send', localized('Send')],
            ['send-and-archive', localized('Send and archive')],
          ]}
        />
        <ConfigSegmentedRow
          keyPath="core.sending.defaultReplyType"
          label={localized('Reply button')}
          options={[
            ['reply', localized('Reply')],
            ['reply-all', localized('Reply all')],
          ]}
        />
        <ConfigSelectRow
          keyPath="core.sending.defaultAccountIdForSend"
          label={localized('New mail from')}
          options={[
            ['selected-mailbox', localized("The account you're in")],
            ...accounts.map((a) => [a.id, a.me().toString()] as [string, string]),
          ]}
        />
        <ConfigSwitchRow
          keyPath="core.sending.sounds"
          label={localized('Play a sound when mail is sent')}
        />
        <SettingsRow
          label={localized('Warnings you turned off')}
          description={localized(
            'Warnings about recipients that you asked Tern not to show again.'
          )}
        >
          <Button variant="outline" onClick={onTurnWarningsBackOn}>
            {localized('Turn back on')}
          </Button>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title={localized('Attachments')}>
        <ConfigSwitchRow
          keyPath="core.attachments.displayFilePreview"
          label={localized('Show previews of attachments')}
        />
        <ConfigSwitchRow
          keyPath="core.attachments.openFolderAfterDownload"
          label={
            process.platform === 'darwin'
              ? localized('Show the file in Finder after downloading')
              : localized('Open the folder after downloading')
          }
        />
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesComposing.displayName = 'PreferencesComposing';
