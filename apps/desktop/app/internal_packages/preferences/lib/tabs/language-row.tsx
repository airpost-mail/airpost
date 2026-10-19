import React from 'react';
import { ipcRenderer } from 'electron';
import { localized, getAvailableLanguages } from 'mailspring-exports';
import { SettingsRow } from 'mailspring-component-kit';

/** The interface language, with a Relaunch button when the choice differs from the running one. */
const LanguageRow = ({ config }: { config: any }) => {
  const { automatic, current, verified, experimental } = getAvailableLanguages();

  const configValue = config.get('core.intl.language');
  const relaunchRequiredToSet = configValue && current.key !== configValue;
  const relaunchRequiredToUnset = !configValue && current.key !== automatic.key;
  const relaunchRequired = relaunchRequiredToSet || relaunchRequiredToUnset;

  return (
    <SettingsRow
      label={localized('Language')}
      description={localized('Tern restarts to switch language.')}
    >
      {relaunchRequired && (
        <button className="btn" onClick={() => ipcRenderer.send('command', 'application:relaunch')}>
          {localized('Relaunch')}
        </button>
      )}
      <select
        className="settings-select"
        aria-label={localized('Language')}
        onChange={(e) => config.set('core.intl.language', e.target.value)}
        value={configValue || ''}
      >
        <option key="auto" value="">
          {localized('Automatic')} ({automatic.name})
        </option>
        <option key="sep1" disabled />
        <option key="subtitle1" disabled>
          {localized('Contributed:')}
        </option>
        {verified.map(({ key, name }) => (
          <option key={key} value={key}>
            {name}
          </option>
        ))}
        <option key="sep2" disabled />
        <option key="subtitle2" disabled>
          {localized('Experimental:')}
        </option>
        {experimental.map(({ key, name }) => (
          <option key={key} value={key}>
            {name}
          </option>
        ))}
      </select>
    </SettingsRow>
  );
};

export default LanguageRow;
