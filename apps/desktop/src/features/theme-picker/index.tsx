import React from 'react';
import * as Actions from '../../data/actions';
import WorkspaceStore from '../../data/stores/workspace';

import ThemePicker from './theme-picker';

let disposable: { dispose: () => void } | null = null;

export function activate() {
  disposable = AppEnv.commands.add(document.body, 'window:launch-theme-picker', () => {
    WorkspaceStore.popToRootSheet();
    Actions.openModal({
      component: <ThemePicker />,
      height: 390,
      width: 250,
    });
  });
}

export function deactivate() {
  if (disposable) disposable.dispose();
  disposable = null;
}
