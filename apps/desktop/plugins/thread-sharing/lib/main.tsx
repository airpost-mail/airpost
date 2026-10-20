import { ComponentRegistry } from 'mailspring-exports';

import plugin from '../package.json';
import ThreadSharingButton from './button';

export const PLUGIN_NAME = plugin.title;
export const PLUGIN_ID = plugin.name;

export function activate() {
  ComponentRegistry.register(ThreadSharingButton, {
    role: 'ThreadActionsToolbarButton',
  });
}

export function deactivate() {
  ComponentRegistry.unregister(ThreadSharingButton);
}
