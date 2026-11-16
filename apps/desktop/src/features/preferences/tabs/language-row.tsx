import React from 'react';
import { ipcRenderer } from 'electron';
import { localized, getAvailableLanguages } from '../../../intl/localize';
import { SettingsRow } from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from '../../../components/kit/select';

// The Automatic choice clears core.intl.language; Radix Select reserves '' for "cleared".
const AUTOMATIC = '__automatic__';

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
      description={localized('Airpost restarts to switch language.')}
    >
      {relaunchRequired && (
        <Button
          variant="outline"
          onClick={() => ipcRenderer.send('command', 'application:relaunch')}
        >
          {localized('Relaunch')}
        </Button>
      )}
      <Select
        value={configValue || AUTOMATIC}
        onValueChange={(key) => config.set('core.intl.language', key === AUTOMATIC ? '' : key)}
      >
        <SelectTrigger aria-label={localized('Language')}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={AUTOMATIC}>
            {localized('Automatic')} ({automatic.name})
          </SelectItem>
          <SelectSeparator />
          <SelectGroup>
            <SelectLabel>{localized('Contributed:')}</SelectLabel>
            {verified.map(({ key, name }) => (
              <SelectItem key={key} value={key}>
                {name}
              </SelectItem>
            ))}
          </SelectGroup>
          <SelectSeparator />
          <SelectGroup>
            <SelectLabel>{localized('Experimental:')}</SelectLabel>
            {experimental.map(({ key, name }) => (
              <SelectItem key={key} value={key}>
                {name}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    </SettingsRow>
  );
};

export default LanguageRow;
