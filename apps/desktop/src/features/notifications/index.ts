/* eslint no-unused-vars:0 */

import { ComponentRegistry, WorkspaceStore } from 'mailspring-exports';
import ActivitySidebar from './sidebar/activity-sidebar';
import NotifWrapper from './wrapper';

import AccountErrorNotification from './items/account-error';
import DefaultClientNotification from './items/default-client';
import DevModeNotification from './items/dev-mode';
import DisabledMailRulesNotification from './items/disabled-mail-rules';
import OfflineNotification from './items/offline';
import UpdateNotification from './items/update';
import { EngineUnavailableNotification } from '../../components/engine-unavailable-state';

const notifications = [
  AccountErrorNotification,
  DefaultClientNotification,
  DevModeNotification,
  DisabledMailRulesNotification,
  EngineUnavailableNotification,
  OfflineNotification,
  UpdateNotification,
];

export function activate() {
  ComponentRegistry.register(ActivitySidebar, { location: WorkspaceStore.Location.RootSidebar });
  ComponentRegistry.register(NotifWrapper, { location: WorkspaceStore.Location.RootSidebar });

  for (const notification of notifications) {
    ComponentRegistry.register(notification, { role: 'RootSidebar:Notifications' });
  }
}

export function serialize() {}

export function deactivate() {
  ComponentRegistry.unregister(ActivitySidebar);
  ComponentRegistry.unregister(NotifWrapper);

  for (const notification of notifications) {
    ComponentRegistry.unregister(notification);
  }
}
