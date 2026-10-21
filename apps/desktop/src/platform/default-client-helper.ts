import fs from 'fs';
import { exec } from 'child_process';
import { shell } from 'electron';
import { localized } from '../intl/localize';
import { DESKTOP_FILE_NAME } from './linux-desktop';
import { App, Dialogs } from './app-api';

interface DCH {
  available(): boolean;
  isRegisteredForURLScheme(scheme: string, callback: (registered: boolean | Error) => void): void;
  resetURLScheme(scheme: string, callback: (error?: Error | null) => void): void;
  registerForURLScheme(scheme: string, callback: (error?: Error | null) => void): void;
}

export class DefaultClientHelperWindows implements DCH {
  available() {
    return true;
  }

  isRegisteredForURLScheme(scheme: string, callback: (registered: boolean | Error) => void) {
    if (!callback) {
      throw new Error('isRegisteredForURLScheme is async, provide a callback');
    }
    let output = '';
    exec(
      `reg.exe query HKCU\\SOFTWARE\\Microsoft\\Windows\\Roaming\\OpenWith\\UrlAssociations\\${scheme}\\UserChoice`,
      (err1, stdout1) => {
        output += stdout1.toString();
        exec(
          `reg.exe query HKCU\\SOFTWARE\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\${scheme}\\UserChoice`,
          (err2, stdout2) => {
            output += stdout2.toString();
            const err = err1 || err2;
            if (err) {
              callback(err);
              return;
            }
            callback(output.includes('Tern.Url.'));
          }
        );
      }
    );
  }

  async resetURLScheme() {
    // On Windows 11 21H2+ (with April 2023 update), we can deep link directly to Tern's
    // default app settings page. On older Windows versions, this falls back to the main
    // Default Apps page, which is still better than opening a web browser.
    shell.openExternal('ms-settings:defaultapps?registeredAppUser=Tern').catch((err) => {
      AppEnv.showErrorDialog({
        title: localized('Failed to Open Settings'),
        message: localized('Tern was unable to open Windows Settings.\n\n%@', err.message),
      });
    });
  }

  registerForURLScheme(scheme: string, callback = (error?: Error | null) => {}) {
    // Ensure that our registry entires are present
    App.registerDefaultMailClient().then(async ({ error, didMakeDefault }) => {
      if (error) {
        await Dialogs.showMessageBox(
          {
            type: 'error',
            buttons: [localized('OK')],
            message: localized('An error has occurred'),
            detail: error,
          },
          'none'
        );
        return;
      }

      if (!didMakeDefault) {
        const { response } = await Dialogs.showMessageBox(
          {
            type: 'info',
            buttons: [localized('Open Settings'), localized('Cancel')],
            defaultId: 0,
            message: localized('Visit Windows Settings to finish making Tern your mail client'),
            detail: localized(
              "Click 'Open Settings' to open Windows Settings where you can set Tern as your default email app."
            ),
          },
          'none'
        );
        if (response === 0) {
          // On Windows 11 21H2+ (with April 2023 update), this deep links directly to
          // Tern's default app settings. On older versions, falls back to Default Apps.
          shell.openExternal('ms-settings:defaultapps?registeredAppUser=Tern').catch((err) => {
            AppEnv.showErrorDialog({
              title: localized('Failed to Open Settings'),
              message: localized('Tern was unable to open Windows Settings.\n\n%@', err.message),
            });
          });
        }
      }
      callback(null);
    });
  }
}

export class DefaultClientHelperLinux implements DCH {
  available() {
    return !process.env.SNAP;
  }

  isRegisteredForURLScheme(scheme: string, callback: (registered: boolean | Error) => void) {
    if (!callback) {
      throw new Error('isRegisteredForURLScheme is async, provide a callback');
    }
    exec(`xdg-mime query default x-scheme-handler/${scheme}`, (err, stdout) =>
      err ? callback(err) : callback(stdout.trim() === DESKTOP_FILE_NAME)
    );
  }

  resetURLScheme(scheme: string, callback = (error?: Error | null) => {}) {
    exec(`xdg-mime default thunderbird.desktop x-scheme-handler/${scheme}`, (err: Error | null) =>
      err ? callback(err) : callback(null)
    );
  }
  registerForURLScheme(scheme: string, callback = (error?: Error | null) => {}) {
    exec(`xdg-mime default ${DESKTOP_FILE_NAME} x-scheme-handler/${scheme}`, (err: Error | null) =>
      err ? callback(err) : callback(null)
    );
  }
}

export class DefaultClientHelperMac implements DCH {
  secure = false;

  available() {
    return true;
  }

  isRegisteredForURLScheme(scheme: string, callback: (registered: boolean) => void) {
    if (!callback) {
      throw new Error('isRegisteredForURLScheme is async, provide a callback');
    }
    App.isDefaultProtocolClient(scheme).then(callback);
  }

  resetURLScheme(scheme: string, callback = (error?: Error | null) => {}) {
    App.removeAsDefaultProtocolClient(scheme).then((success) =>
      callback(success ? null : new Error(`Failed to remove Tern as default handler for ${scheme}`))
    );
  }

  registerForURLScheme(scheme: string, callback = (error?: Error | null) => {}) {
    App.setAsDefaultProtocolClient(scheme).then((success) =>
      callback(success ? null : new Error(`Failed to set Tern as default handler for ${scheme}`))
    );
  }
}

let Default:
  | typeof DefaultClientHelperMac
  | typeof DefaultClientHelperLinux
  | typeof DefaultClientHelperWindows;

if (process.platform === 'darwin') {
  Default = DefaultClientHelperMac;
} else if (process.platform === 'win32') {
  Default = DefaultClientHelperWindows;
} else {
  Default = DefaultClientHelperLinux;
}
export const DefaultClientHelper = Default;
