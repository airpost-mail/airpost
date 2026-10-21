import SystemTrayIconStore from './icon-store';

let store: SystemTrayIconStore | null = null;

export function activate() {
  store = new SystemTrayIconStore();
  store.activate();
}

export function deactivate() {
  if (store) store.deactivate();
  store = null;
}

export function serialize() {}
