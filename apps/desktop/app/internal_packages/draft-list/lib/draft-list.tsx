import React from 'react';
import {
  Actions,
  DOMUtils,
  Message,
  WorkspaceStore,
  FocusedContentStore,
} from 'mailspring-exports';
import {
  FluxContainer,
  FocusContainer,
  EmptyListState,
  MultiselectList,
} from 'mailspring-component-kit';
import DraftListStore from './draft-list-store';
import * as DraftListColumns from './draft-list-columns';

class DraftList extends React.Component {
  static displayName = 'DraftList';
  static containerRequired = false;
  static containerStyles = {
    minWidth: DOMUtils.getWorkspaceCssNumberProperty('thread-list-min-width', 100),
    maxWidth: DOMUtils.getWorkspaceCssNumberProperty('thread-list-max-width', 3000),
  };

  render() {
    return (
      <FluxContainer
        stores={[DraftListStore]}
        getStateFromStores={() => {
          return { dataSource: DraftListStore.dataSource() };
        }}
      >
        <FocusContainer collection="draft">
          <MultiselectList
            className="draft-list"
            columns={DraftListColumns.Wide}
            onDoubleClick={this._onDoubleClick}
            EmptyComponent={EmptyListState}
            keymapHandlers={this._keymapHandlers()}
            itemPropsProvider={this._itemPropsProvider}
            itemHeight={39}
          />
        </FocusContainer>
      </FluxContainer>
    );
  }
  _itemPropsProvider = (draft: Message) => {
    const props: any = {};
    if ((draft as any).uploadTaskId) {
      props.className = 'sending';
    }
    return props;
  };

  _keymapHandlers = () => {
    return {
      'core:focus-item': this._onEnter,
      'core:delete-item': this._onRemoveFromView,
      'core:gmail-remove-from-view': this._onRemoveFromView,
      'core:remove-from-view': this._onRemoveFromView,
    };
  };

  _onDoubleClick = (draft: Message) => {
    if (!(draft as any).uploadTaskId) {
      Actions.composePopoutDraft(draft.headerMessageId);
    }
  };

  // In split mode the draft is already open beside the list, so Enter moves into it.
  _onEnter = () => {
    if (WorkspaceStore.layoutMode() !== 'list' && DraftListStore.focusPreview()) {
      return;
    }
    const cursor = FocusedContentStore.keyboardCursor('draft');
    if (cursor) {
      Actions.setFocus({ collection: 'draft', item: cursor });
    }
  };

  _onRemoveFromView = () => {
    for (const draft of DraftListStore.draftsForCommand()) {
      Actions.destroyDraft(draft);
    }
  };
}

export default DraftList;
