/*
 * decaffeinate suggestions:
 * DS205: Consider reworking code to avoid use of IIFEs
 * DS207: Consider shorter variations of null checks
 * Full docs: https://github.com/decaffeinate/decaffeinate/blob/master/docs/suggestions.md
 */
import { BrowserWindow, Menu, app } from 'electron';
import { developerToolsAvailable } from '../../platform/developer-tools';
const Utils = require('../../data/models/utils');
const { localized } = require('../../intl/localize');

// A menu item as the windows send it (window/menu-manager.ts) or getDefaultTemplate builds it:
// Electron's options plus the command it sends and our metadata.
export interface MenuTemplateItem extends Electron.MenuItemConstructorOptions {
  command?: string;
  args?: unknown;
  metadata?: { [key: string]: unknown };
  submenu?: MenuTemplateItem[];
}

// Electron's MenuItem keeps the template's other properties, so the built items carry these too.
interface NativeMenuItem extends Electron.MenuItem {
  command?: string;
  metadata?: { [key: string]: unknown };
}

// Used to manage the global application menu.
//
// It's created by {Application} upon instantiation and used to add, remove
// and maintain the state of all menu items.
export default class ApplicationMenu {
  windowTemplates = new WeakMap<BrowserWindow, MenuTemplateItem[]>();
  version: string;
  lastFocusedWindow: BrowserWindow | null = null;
  activeTemplate?: MenuTemplateItem[];
  // Built by the constructor's setActiveTemplate.
  menu!: Electron.Menu;

  constructor(version: string) {
    this.version = version;
    this.setActiveTemplate(this.getDefaultTemplate());
    global.application.autoUpdateManager.on('state-changed', (state: string) => {
      this.updateAutoupdateMenuItem(state);
    });
  }

  // Public: Updates the entire menu with the given keybindings.
  //
  // window - The BrowserWindow this menu template is associated with.
  // template - The Object which describes the menu to display.
  // keystrokesByCommand - An Object where the keys are commands and the values
  //                       are Arrays containing the keystroke.
  update(
    window: BrowserWindow,
    template: MenuTemplateItem[],
    keystrokesByCommand: Record<string, string[]>
  ) {
    this.translateTemplate(template, keystrokesByCommand);
    this.windowTemplates.set(window, template);
    if (window === this.lastFocusedWindow) {
      this.setActiveTemplate(template);
    }
  }

  setActiveTemplate(template: MenuTemplateItem[]) {
    if (!Utils.isEqual(template, this.activeTemplate)) {
      this.activeTemplate = template;
      this.rebuildMenuWithActiveTemplate();
    }
  }

  rebuildMenuWithActiveTemplate() {
    const fullTemplate = Utils.deepClone(this.activeTemplate);
    this.extendTemplateWithVersion(fullTemplate);
    this.extendTemplateWithWindowMenu(fullTemplate);

    this.menu = Menu.buildFromTemplate(fullTemplate);
    Menu.setApplicationMenu(this.menu);

    this.updateAutoupdateMenuItem(global.application.autoUpdateManager.getState());
    this.updateFullscreenMenuItem(
      this.lastFocusedWindow != null && this.lastFocusedWindow.isFullScreen()
    );
    this.updateDevModeItem();
  }

  // Register a BrowserWindow with this application menu.
  addWindow(window: BrowserWindow) {
    if (!this.lastFocusedWindow) {
      this.lastFocusedWindow = window;
    }

    const focusHandler = () => {
      this.lastFocusedWindow = window;
      const template = this.windowTemplates.get(window);
      if (template) {
        this.setActiveTemplate(template);
      }
    };

    window.on('focus', focusHandler);
    window.on('enter-full-screen', focusHandler);
    window.on('leave-full-screen', focusHandler);
    window.once('closed', () => {
      if (window === this.lastFocusedWindow) {
        this.lastFocusedWindow = null;
      }
      this.windowTemplates.delete(window);
      this.rebuildMenuWithActiveTemplate();
      window.removeListener('focus', focusHandler);
      window.removeListener('enter-full-screen', focusHandler);
      window.removeListener('leave-full-screen', focusHandler);
    });

    this.rebuildMenuWithActiveTemplate();
    this.enableWindowSpecificItems(true);
  }

  // Flattens the given menu and submenu items into an single Array.
  //
  // menu - A complete menu configuration object for electron's menu API.
  //
  // Returns an Array of native menu items.
  flattenMenuItems(menu: Electron.Menu): NativeMenuItem[] {
    let items: NativeMenuItem[] = [];
    for (const item of menu.items || []) {
      items.push(item);
      if (item.submenu) {
        items = items.concat(this.flattenMenuItems(item.submenu));
      }
    }
    return items;
  }

  // Flattens the given menu template into an single Array.
  //
  // template - An object describing the menu item.
  //
  // Returns an Array of native menu items.
  flattenMenuTemplate(template: MenuTemplateItem[]): MenuTemplateItem[] {
    let items: MenuTemplateItem[] = [];
    for (const item of template) {
      items.push(item);
      if (item.submenu) {
        items = items.concat(this.flattenMenuTemplate(item.submenu));
      }
    }
    return items;
  }

  // Public: Used to make all window related menu items are active.
  //
  // enable - If true enables all window specific items, if false disables all
  //          window specific items.
  enableWindowSpecificItems(enable: boolean) {
    for (const item of this.flattenMenuItems(this.menu)) {
      if (item.metadata && item.metadata['windowSpecific']) {
        item.enabled = enable;
      }
    }
  }

  // Replaces VERSION with the current version.
  extendTemplateWithVersion(template: MenuTemplateItem[]) {
    const item = this.flattenMenuTemplate(template).find(({ label }) => label === 'VERSION');
    if (item) {
      item.label = `Version ${this.version}`;
    }
  }

  extendTemplateWithWindowMenu(template: MenuTemplateItem[]) {
    const windowMenu = template.find(({ label }) => label === localized('Window'));
    if (!windowMenu || !windowMenu.submenu) {
      return;
    }
    const idx = windowMenu.submenu.findIndex(({ id }) => id === 'window-list-separator');

    const windows = global.application.windowManager.getOpenWindows();
    const windowsItems = windows
      .filter((w) => w.windowType !== 'default' && w.windowType !== 'contacts')
      .map((w) => ({
        label: w.loadSettings().title || 'Window',
        click() {
          w.show();
          w.focus();
        },
      }));

    return windowMenu.submenu.splice(idx, 0, { type: 'separator' as const }, ...windowsItems);
  }

  // Sets the proper visible state the update menu items
  updateAutoupdateMenuItem(state: string) {
    const checkForUpdateItem = this.flattenMenuItems(this.menu).find(
      ({ label }) => label === localized('Check for Updates')
    );
    const downloadingUpdateItem = this.flattenMenuItems(this.menu).find(
      ({ label }) => label === localized('Downloading Update')
    );
    const installUpdateItem = this.flattenMenuItems(this.menu).find(
      ({ label }) => label === localized('Restart and Install Update')
    );

    if (checkForUpdateItem == null || downloadingUpdateItem == null || installUpdateItem == null) {
      return;
    }

    checkForUpdateItem.visible = false;
    downloadingUpdateItem.visible = false;
    installUpdateItem.visible = false;

    switch (state) {
      case 'idle':
      case 'error':
      case 'no-update-available':
        checkForUpdateItem.visible = true;
        break;
      case 'checking':
      case 'downloading':
        downloadingUpdateItem.visible = true;
        break;
      case 'update-available':
        installUpdateItem.visible = true;
        break;
      default:
    }
  }

  updateFullscreenMenuItem(fullscreen: boolean) {
    const enterItem = this.flattenMenuItems(this.menu).find(
      ({ label }) => label === localized('Enter Full Screen')
    );
    const exitItem = this.flattenMenuItems(this.menu).find(
      ({ label }) => label === localized('Exit Full Screen')
    );
    if (!enterItem || !exitItem) {
      return;
    }
    enterItem.visible = !fullscreen;
    exitItem.visible = fullscreen;
  }

  updateDevModeItem() {
    const devModeItem = this.flattenMenuItems(this.menu).find(
      ({ command }) => command === 'application:toggle-dev'
    );
    if (devModeItem) {
      devModeItem.checked = global.application.devMode;
    }
  }

  // Default list of menu items.
  //
  // Returns an Array of menu item Objects.
  getDefaultTemplate(): MenuTemplateItem[] {
    return [
      {
        label: 'Airpost',
        submenu: [
          { label: localized('Check for Updates'), metadata: { autoUpdate: true } },
          {
            label: localized('Reload'),
            accelerator: 'CmdOrCtrl+R',
            click: () => BrowserWindow.getFocusedWindow()?.reload(),
          },
          {
            label: localized('Close Window'),
            accelerator: 'CmdOrCtrl+Shift+W',
            click: () => BrowserWindow.getFocusedWindow()?.close(),
          },
          ...(developerToolsAvailable()
            ? [
                {
                  label: localized('Toggle Dev Tools'),
                  accelerator: 'CmdOrCtrl+Alt+I',
                  click: () => BrowserWindow.getFocusedWindow()?.webContents.toggleDevTools(),
                },
              ]
            : []),
          {
            label: localized('Quit'),
            accelerator: 'CmdOrCtrl+Q',
            click() {
              app.quit();
            },
          },
        ],
      },
    ];
  }

  // Combines a menu template with the appropriate keystroke.
  //
  // template - An Object conforming to electron's menu api but lacking
  //            accelerator and click properties.
  // keystrokesByCommand - An Object where the keys are commands and the values
  //                       are Arrays containing the keystroke.
  //
  // Returns a complete menu configuration object for electron's menu API.
  //
  translateTemplate(template: MenuTemplateItem[], keystrokesByCommand: Record<string, string[]>) {
    template.forEach((item) => {
      const metadata = item.metadata || {};
      item.metadata = metadata;
      const command = item.command;
      if (command) {
        item.accelerator = this.acceleratorForCommand(command, keystrokesByCommand);
        item.click = () => {
          global.application.sendCommand(command, item.args);
        };
        if (!/^application:/.test(command)) {
          metadata['windowSpecific'] = true;
        }
      }
      if (item.submenu) {
        this.translateTemplate(item.submenu, keystrokesByCommand);
      }
    });
    return template;
  }

  // Determine the accelerator for a given command.
  //
  // command - The name of the command.
  // keystrokesByCommand - An Object where the keys are commands and the values
  //                       are Arrays containing the keystroke.
  //
  // Returns a String containing the keystroke in a format that can be interpreted
  //   by Electron to provide nice icons where available.
  acceleratorForCommand(command: string, keystrokesByCommand: Record<string, string[]>) {
    // Outlook binds quit to ["command+q", "alt+f4"] so that Cmd+Q survives on macOS
    // while ctrl+q is free for mark-as-read elsewhere. On Windows and Linux a
    // `command` keystroke is the Super/Win key, so prefer a native keystroke for
    // the menu and only fall back to it when nothing else is bound.
    const keystrokes = keystrokesByCommand[command] || [];
    const native =
      process.platform === 'darwin'
        ? keystrokes
        : keystrokes.filter((keystroke) => !/\b(command|meta)\b/.test(keystroke));
    let firstKeystroke = native[0] || keystrokes[0];
    if (!firstKeystroke) {
      return undefined;
    }

    if (/f\d+/.test(firstKeystroke)) {
      firstKeystroke = firstKeystroke.toUpperCase();
    }

    const parts = firstKeystroke.split('+');
    const key = parts[parts.length - 1];

    const modifiers = parts.slice(0, -1).map((modifier) =>
      modifier
        .replace(/shift/gi, 'Shift')
        .replace(/command/gi, 'Command')
        .replace(/mod/gi, 'CmdOrCtrl')
        .replace(/ctrl/gi, 'Ctrl')
        .replace(/alt/gi, 'Alt')
    );

    const keys = modifiers.concat([key.toUpperCase()]);
    return keys.join('+');
  }
}
