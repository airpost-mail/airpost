import React from 'react';
import { ipcRenderer } from 'electron';
import {
  localized,
  ComponentRegistry,
  WorkspaceStore,
  FocusedPerspectiveStore,
} from 'mailspring-exports';
import { RetinaImg } from './retina-img';
import { InjectedComponentSet } from './injected/component-set';

export const SHOW_NAV_RAIL_CONFIG_KEY = 'core.workspace.showNavRail';

export function isNavRailShown() {
  return AppEnv.config.get(SHOW_NAV_RAIL_CONFIG_KEY) !== false;
}

export function setNavRailShown(shown: boolean) {
  AppEnv.config.set(SHOW_NAV_RAIL_CONFIG_KEY, shown);
}

/**
 * Registers whichever of the View menu's Show / Hide Sections Bar commands applies. Both menu
 * items are `hideWhenDisabled`, so registering one at a time picks the label the menu shows.
 */
export function registerNavRailCommands(shown: boolean) {
  return AppEnv.commands.add(
    document.body,
    shown
      ? { 'navigation:hide-nav-rail': () => setNavRailShown(false) }
      : { 'navigation:show-nav-rail': () => setNavRailShown(true) }
  );
}

interface NavRailItemProps {
  label: string;
  iconName: string;
  /** An application: command, so the rail, the Window menu and the shortcut take one path. */
  command: string;
  isActive: () => boolean;
}

export class NavRailItem extends React.Component<NavRailItemProps, { active: boolean }> {
  static displayName = 'NavRailItem';

  _unlisten?: () => void;

  state = { active: this.props.isActive() };

  componentDidMount() {
    this._unlisten = WorkspaceStore.listen(() => this.setState({ active: this.props.isActive() }));
  }

  componentWillUnmount() {
    this._unlisten?.();
  }

  render() {
    const { label, iconName, command } = this.props;
    const { active } = this.state;
    return (
      <button
        className={`btn-icon nav-rail-item${active ? ' active' : ''}`}
        title={label}
        aria-label={label}
        aria-current={active ? 'page' : undefined}
        onClick={() => ipcRenderer.send('command', command)}
      >
        <RetinaImg name={iconName} mode={RetinaImg.Mode.ContentIsMask} />
      </button>
    );
  }
}

// Part of the rail rather than registered by the contacts package: that package loads only in the
// Contacts window, which application:show-contacts opens or focuses. The rail lives in the main
// window, so this item never shows as active. Rendered after the registered items to keep Mail and
// Calendar in the Window menu's order.
export const ContactsNavRailItem = () => (
  <NavRailItem
    label={localized('Contacts')}
    iconName="person.png"
    command="application:show-contacts"
    isActive={() => false}
  />
);
ContactsNavRailItem.displayName = 'ContactsNavRailItem';

/** Plugins add a section with the `NavRail:Item` role. */
export class NavRail extends React.Component {
  static displayName = 'NavRail';

  render() {
    const hideLabel = localized('Hide Sections Bar');
    return (
      <nav className="nav-rail" aria-label={localized('Sections')}>
        <InjectedComponentSet
          matching={{ role: 'NavRail:Item' }}
          direction="column"
          height="auto"
          containersRequired={false}
        />
        <ContactsNavRailItem />
        <div className="nav-rail-footer">
          <div className="nav-rail-divider" role="separator" />
          <button
            className="btn-icon nav-rail-item nav-rail-toggle"
            title={hideLabel}
            aria-label={hideLabel}
            onClick={() => setNavRailShown(false)}
          >
            <RetinaImg name="toolbar-icon-toggle-pane.png" mode={RetinaImg.Mode.ContentIsMask} />
          </button>
        </div>
      </nav>
    );
  }
}

/** Stays in the window's bottom-left corner while the rail is hidden. */
export const NavRailRestoreButton = () => {
  const label = localized('Show Sections Bar');
  return (
    <button
      className="btn-icon nav-rail-restore"
      title={label}
      aria-label={label}
      onClick={() => setNavRailShown(true)}
    >
      <RetinaImg name="toolbar-icon-toggle-pane.png" mode={RetinaImg.Mode.ContentIsMask} />
    </button>
  );
};
NavRailRestoreButton.displayName = 'NavRailRestoreButton';

export const MailNavRailItem = () => (
  <NavRailItem
    label={localized('Mail')}
    iconName="inbox.png"
    command="application:show-mail"
    isActive={() =>
      FocusedPerspectiveStore.isShowingMail() &&
      WorkspaceStore.topSheet() !== WorkspaceStore.Sheet.Preferences
    }
  />
);
MailNavRailItem.displayName = 'MailNavRailItem';

ComponentRegistry.register(MailNavRailItem, { role: 'NavRail:Item' });
