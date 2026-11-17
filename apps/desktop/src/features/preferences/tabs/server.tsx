import React from 'react';
import { shell } from 'electron';
import { localized } from '../../../intl/localize';
import { RetinaImg } from '../../../components/retina-img';
import { SettingsPage, SettingsSection, SettingsNote } from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import { TextField } from '../../../components/kit/text-field';

const SELF_HOSTING_GUIDE = 'https://docs.airpost.sh/self-hosting/overview';

const FEATURES: { title: () => string; detail: () => string }[] = [
  {
    title: () => localized('Airpost on the web'),
    detail: () => localized('Your mail in any browser.'),
  },
  {
    title: () => localized('Always on'),
    detail: () => localized('Scheduled sends and rules run while your computers sleep.'),
  },
  {
    title: () => localized('Read receipts and link tracking'),
    detail: () => localized('See when mail is opened and links are clicked.'),
  },
  {
    title: () => localized('Settings everywhere'),
    detail: () => localized('Signatures, templates, rules and snoozes on every computer.'),
  },
];

/** An Airpost server adds the web client, always-on work and sync.
 * Signing in from the desktop isn't built yet, so the page explains and links the guide. */
export default function PreferencesServer() {
  return (
    <SettingsPage title="" className="preferences-server">
      <SettingsSection bare>
        <div className="settings-card server-empty">
          <div className="server-art">
            <RetinaImg
              name="airpost.png"
              mode={RetinaImg.Mode.ContentPreserve}
              style={{ width: 48, height: 48 }}
            />
          </div>
          <h3>{localized('Connect an Airpost server')}</h3>
          <p>
            {localized(
              'An Airpost server you run gives you Airpost on the web, sends that go out while this computer is off, read receipts, and settings that follow you to your other computers. Your mail keeps syncing here too.'
            )}
          </p>
          <div className="server-connect">
            <TextField
              type="url"
              placeholder="airpost.example.com"
              aria-label={localized('Server address')}
              disabled
            />
            <Button variant="default" disabled>
              {localized('Sign in')}
            </Button>
          </div>
          <SettingsNote>
            {localized('Signing in from the desktop comes in a later version.')}{' '}
            <a onClick={() => shell.openExternal(SELF_HOSTING_GUIDE)}>
              {localized('How to set up an Airpost server')}
            </a>
          </SettingsNote>
        </div>
      </SettingsSection>

      <SettingsSection title={localized('What a server adds')}>
        <div className="server-features">
          {FEATURES.map((f) => (
            <div key={f.title()}>
              <b>{f.title()}</b>
              <small>{f.detail()}</small>
            </div>
          ))}
        </div>
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesServer.displayName = 'PreferencesServer';
