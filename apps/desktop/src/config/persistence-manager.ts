import path from 'path';
import fs from 'fs';
import { BrowserWindow, dialog, app } from 'electron';
import { atomicWriteFileSync } from '../shared/fs-utils';
import { localized } from '../shared/intl';
import { ConfigChannel, ConfigReply } from './channels';
import {
  ENGINE_SETTINGS_KEYS,
  EngineSettingsMirror,
  LANGUAGE_KEY,
  SettingsHost,
  touchesKey,
} from '../main/engine/settings';
import { USE_24_HOUR_KEY } from '../shared/date-format';

let _ = require('underscore');
_ = Object.assign(_, require('./utils'));

const RETRY_SAVES = 3;

// Tells the main process's Config and every window but the one that made the change.
type ChangeListener = (settings: object, sourceWebcontentsId?: number) => void;

export default class ConfigPersistenceManager implements SettingsHost {
  userWantsToPreserveErrors = false;
  // Set once the profile is being reset: a later write would put the old settings back.
  discarded = false;
  saveRetries = 0;
  settings = {};
  configDirPath: string;
  configFilePath: string;
  resourcePath: string;
  lastSaveTimestamp?: number;
  // Set by the application when a `serve` engine is supervised (src/main/engine/settings.ts).
  engineSettings: EngineSettingsMirror | null = null;
  onChange: ChangeListener;

  constructor({
    configDirPath,
    resourcePath,
    onChange = notifyConfigAndWindows,
  }: {
    configDirPath: string;
    resourcePath: string;
    onChange?: ChangeListener;
  }) {
    this.configDirPath = configDirPath;
    this.resourcePath = resourcePath;
    this.onChange = onChange;

    this.configFilePath = path.join(this.configDirPath, 'config.json');

    this.initializeConfigDirectory();
    this.load();
  }

  initializeConfigDirectory() {
    if (!fs.existsSync(this.configDirPath)) {
      fs.mkdirSync(this.configDirPath, { recursive: true });
      const templateConfigDirPath = path.join(this.resourcePath, 'default-profile');
      fs.cpSync(templateConfigDirPath, this.configDirPath, { recursive: true });
    }

    try {
      const stat = fs.statSync(this.configDirPath);
      const configDirPathAccessMode = (stat.mode & parseInt('777', 8)).toString(8);
      if (configDirPathAccessMode !== '700') {
        fs.chmodSync(this.configDirPath, '700');
      }
    } catch (err) {
      // ignore
    }

    if (!fs.existsSync(this.configFilePath)) {
      this.writeTemplateConfigFile();
    }
  }

  writeTemplateConfigFile() {
    const templateConfigPath = path.join(this.resourcePath, 'default-profile', 'config.json');
    const templateConfig = fs.readFileSync(templateConfigPath).toString();
    fs.writeFileSync(this.configFilePath, templateConfig);
  }

  _showLoadErrorDialog(error) {
    const message = localized(`Failed to load "%@"`, path.basename(this.configFilePath));
    let detail = error.message;

    if (error instanceof SyntaxError) {
      detail += `\n\nThe file ${this.configFilePath} has incorrect JSON formatting or is empty. Fix the formatting to resolve this error, or reset your settings to continue using Tern.`;
    } else {
      detail += `\n\nWe were unable to read the file ${this.configFilePath}. Make sure you have permissions to access this file, and check that the file is not open or being edited and try again.`;
    }

    const clickedIndex = dialog.showMessageBoxSync({
      type: 'error',
      message,
      detail,
      buttons: [localized('Quit'), localized('Try Again'), localized('Reset Configuration')],
    });

    // On Windows, `cancelId` is ignored by Electron, so dismissing the dialog without
    // clicking a button (eg: via the window's close button, Alt+F4, or Esc) can return
    // an index outside 0-2. Treat that the same as "Quit" rather than throwing, since
    // this runs synchronously during app startup and an uncaught error here prevents
    // the app from launching at all.
    return ['quit', 'tryagain', 'reset'][clickedIndex] || 'quit';
  }

  load() {
    this.userWantsToPreserveErrors = false;

    try {
      const json = JSON.parse(fs.readFileSync(this.configFilePath).toString());
      if (!json || !json['*']) {
        throw new Error('config json appears empty');
      }
      this.settings = json['*'];
      this.emitChangeEvent();
    } catch (error) {
      error.message = localized(`Failed to load config.json: %@`, error.message);

      const action = this._showLoadErrorDialog(error);
      if (action === 'quit') {
        this.userWantsToPreserveErrors = true;
        app.quit();
        return;
      }

      if (action === 'tryagain') {
        this.load();
        return;
      }

      if (action !== 'reset') {
        throw new Error(`Unknown action: ${action}`);
      }

      if (fs.existsSync(this.configFilePath)) {
        fs.unlinkSync(this.configFilePath);
      }
      this.writeTemplateConfigFile();
      this.load();
    }
  }

  _showSaveErrorDialog() {
    const clickedIndex = dialog.showMessageBoxSync({
      type: 'error',
      message: localized(`Failed to save "%@"`, path.basename(this.configFilePath)),
      detail: `\n\nWe were unable to save the file ${this.configFilePath}. Make sure you have permissions to access this file, and check that the file is not open or being edited and try again.`,
      buttons: [localized('Okay'), localized('Try Again')],
    });
    return ['ignore', 'retry'][clickedIndex];
  }

  save = () => {
    if (this.userWantsToPreserveErrors || this.discarded) {
      return;
    }
    const allSettings = { '*': this.settings };
    const allSettingsJSON = JSON.stringify(allSettings, null, 2);
    this.lastSaveTimestamp = Date.now();

    try {
      atomicWriteFileSync(this.configFilePath, allSettingsJSON);
      this.saveRetries = 0;
    } catch (error) {
      if (this.saveRetries >= RETRY_SAVES) {
        error.message = localized(`Failed to save config.json: %@`, error.message);
        const action = this._showSaveErrorDialog();
        this.saveRetries = 0;

        if (action === 'retry') {
          this.save();
        }
        return;
      }

      this.saveRetries++;
      this.save();
    }
  };

  getRawValuesString = () => {
    if (!this.settings || _.isEmpty(this.settings)) {
      throw new Error('this.settings is empty');
    }
    return JSON.stringify(this.settings);
  };

  setSettings = (value, sourceWebcontentsId) => {
    this._forwardEngineKeys(null, () => {
      this.settings = value;
    });
    this.emitChangeEvent({ sourceWebcontentsId });
    this.save();
  };

  setRawValue = (keyPath, value, sourceWebcontentsId) => {
    if (!keyPath) {
      throw new Error('keyPath must not be false-y!');
    }
    this._forwardEngineKeys(keyPath, () => {
      _.setValueForKeyPath(this.settings, keyPath, value);
    });
    this.emitChangeEvent({ sourceWebcontentsId });
    this.save();
  };

  get = (keyPath: string) => _.valueForKeyPath(this.settings, keyPath);

  // Values from the engine's settings store: every window gets them, the engine doesn't.
  applyEngineValues = (changes: { [keyPath: string]: unknown }) => {
    for (const keyPath of Object.keys(changes)) {
      _.setValueForKeyPath(this.settings, keyPath, changes[keyPath]);
    }
    this.emitChangeEvent();
    this.save();
  };

  // Runs `write`, then hands the engine-owned keys whose value it changed to the mirror.
  _forwardEngineKeys(keyPath: string | null, write: () => void) {
    const keys = this.engineSettings
      ? ENGINE_SETTINGS_KEYS.filter((key) => touchesKey(keyPath, key))
      : [];
    const language = this.engineSettings && touchesKey(keyPath, LANGUAGE_KEY);
    const languageBefore = language ? this.get(LANGUAGE_KEY) : undefined;
    const clock = this.engineSettings && touchesKey(keyPath, USE_24_HOUR_KEY);
    const clockBefore = clock ? this.get(USE_24_HOUR_KEY) : undefined;
    const before = keys.map((key) => JSON.stringify(this.get(key)));
    write();
    const changed = keys.filter((key, i) => JSON.stringify(this.get(key)) !== before[i]);
    if (changed.length) this.engineSettings.localChanged(changed);
    if (language && this.get(LANGUAGE_KEY) !== languageBefore)
      this.engineSettings.languageChanged();
    else if (clock && this.get(USE_24_HOUR_KEY) !== clockBefore) this.engineSettings.clockChanged();
  }

  emitChangeEvent = ({ sourceWebcontentsId }: { sourceWebcontentsId?: number } = {}) => {
    this.onChange(this.settings, sourceWebcontentsId);
  };
}

function notifyConfigAndWindows(settings: object, sourceWebcontentsId?: number) {
  global.application.config.updateSettings(settings);

  BrowserWindow.getAllWindows().forEach((win) => {
    if (win.webContents && win.webContents.id !== sourceWebcontentsId) {
      win.webContents.send(ConfigChannel.Reloaded, settings);
    }
  });
}

// Every reply is set, whatever happens: a sendSync without one blocks its window for good.
function replyTo(event: Electron.IpcMainEvent, work: () => string | void) {
  try {
    const value = work();
    event.returnValue = (value === undefined ? {} : { value }) as ConfigReply;
  } catch (err) {
    event.returnValue = { error: (err && err.message) || `${err}` } as ConfigReply;
  }
}

// The windows' Config reads and writes config.json through these (src/config/channels.ts).
export function registerConfigIPC(
  ipcMain: Pick<Electron.IpcMain, 'on'>,
  manager: ConfigPersistenceManager
) {
  ipcMain.on(ConfigChannel.GetRaw, (event) => replyTo(event, () => manager.getRawValuesString()));
  ipcMain.on(ConfigChannel.SetRaw, (event, keyPath: string, value: unknown) =>
    replyTo(event, () => manager.setRawValue(keyPath, value, event.sender.id))
  );
  ipcMain.on(ConfigChannel.SetAll, (event, settings: object) =>
    replyTo(event, () => manager.setSettings(settings, event.sender.id))
  );
}
