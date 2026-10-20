import { localized, WorkspaceStore, Actions } from 'mailspring-exports';
import React from 'react';
import { IconButton } from '../../components/kit/icon-button';

export default class ModeToggle extends React.Component<
  Record<string, unknown>,
  { hidden: boolean }
> {
  static displayName = 'ModeToggle';

  _mounted = false;
  _unsubscriber: () => void;
  column = WorkspaceStore.Location.MessageListSidebar;

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this._unsubscriber = WorkspaceStore.listen(this._onStateChanged);
    this._mounted = true;
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unsubscriber) {
      this._unsubscriber();
    }
  }

  _getStateFromStores() {
    return {
      hidden: WorkspaceStore.isLocationHidden(this.column),
    };
  }

  _onStateChanged = () => {
    // We need to keep track of this because our parent unmounts us in the same
    // event listener cycle that we receive the event in. ie:
    //
    //   for listener in listeners
    //      # 1. workspaceView remove left column
    //      # ---- Mode toggle unmounts, listeners array mutated in place
    //      # 2. ModeToggle update
    if (!this._mounted) {
      return;
    }
    this.setState(this._getStateFromStores());
  };

  _onToggleMode = () => {
    Actions.toggleWorkspaceLocationHidden(this.column);
  };

  render() {
    const label = this.state.hidden ? localized('Show Sidebar') : localized('Hide Sidebar');
    return (
      <IconButton
        variant="toolbar"
        className={`mode-toggle mode-${this.state.hidden}`}
        style={{ order: 500 }}
        label={label}
        icon="toolbar-person-sidebar.png"
        onClick={this._onToggleMode}
      />
    );
  }
}
