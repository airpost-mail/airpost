import React from 'react';
import { localizedReactFragment, localized } from '../../intl/localize';
import ComponentRegistry from '../../packages/component-registry';
import WorkspaceStore from '../../data/stores/workspace';
import HasTutorialTip from '../../components/decorators/has-tutorial-tip';

import ThreadSearchBar from './bar';

const ThreadSearchBarWithTip = HasTutorialTip(ThreadSearchBar, {
  title: localized('Search with ease'),
  instructions: (
    <span>
      {localizedReactFragment(
        `Combine your search queries with Gmail-style terms like %@ and %@ to find anything in your mailbox.`,
        <strong>in: folder</strong>,
        <strong>since: "last month"</strong>
      )}
    </span>
  ),
});

export function activate() {
  ComponentRegistry.register(ThreadSearchBarWithTip, {
    location: WorkspaceStore.Location.ThreadList.Toolbar,
  });
}

export function deactivate() {
  ComponentRegistry.unregister(ThreadSearchBarWithTip);
}
