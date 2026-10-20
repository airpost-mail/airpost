import SystemTrayIconStore from './icon-store';

export function activate() {
  this.store = new SystemTrayIconStore();
  this.store.activate();
}

export function deactivate() {
  this.store.deactivate();
}

export function serialize() {}
