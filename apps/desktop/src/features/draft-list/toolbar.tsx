import React, { Component } from 'react';
import {
  ListensToObservable,
  MultiselectToolbar,
  InjectedComponentSet,
} from 'mailspring-component-kit';

import DraftListStore from './store';
import { DraftDeleteButton } from './toolbar-buttons';
import { Message } from 'mailspring-exports';

function getObservable() {
  return DraftListStore.selectionObservable();
}

function getStateFromObservable(items) {
  if (!items) {
    return { items: [] };
  }
  return { items };
}

class DraftListToolbar extends Component<{ items: Message[] }> {
  static displayName = 'DraftListToolbar';

  onClearSelection = () => {
    DraftListStore.dataSource().selection.clear();
  };

  render() {
    const { selection } = DraftListStore.dataSource();
    const { items } = this.props;

    // The delete button is built in; the role stays for plugins' buttons.
    const toolbarElement = (
      <InjectedComponentSet
        matching={{ role: 'DraftActionsToolbarButton' }}
        exposedProps={{ selection, items }}
      >
        <DraftDeleteButton selection={selection} />
      </InjectedComponentSet>
    );

    return (
      <MultiselectToolbar
        collection="draft"
        selectionCount={items.length}
        toolbarElement={toolbarElement}
        onClearSelection={this.onClearSelection}
      />
    );
  }
}

export default ListensToObservable(DraftListToolbar, { getObservable, getStateFromObservable });
