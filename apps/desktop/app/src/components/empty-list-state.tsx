import _ from 'underscore';
import React from 'react';
import classNames from 'classnames';
import { RetinaImg } from './retina-img';
import {
  localized,
  MailboxPerspective,
  FolderSyncProgressStore,
  FocusedPerspectiveStore,
} from 'mailspring-exports';
import { SyncingListState } from 'mailspring-component-kit';

type EmptyPerspectiveStateProps = {
  perspective?: MailboxPerspective;
  messageContent?: React.ReactNode;
};

class EmptyPerspectiveState extends React.Component<EmptyPerspectiveStateProps> {
  static displayName = 'EmptyPerspectiveState';

  render() {
    const { messageContent, perspective } = this.props;
    let name = perspective.categoriesSharedRole();
    if (perspective.isArchive()) {
      name = 'archive';
    }
    if (!name) {
      ({ name } = perspective);
    }
    if (name) {
      name = name.toLowerCase();
    }

    return (
      <div className="perspective-empty-state">
        {name && (
          <RetinaImg name={`ic-emptystate-${name}.png`} mode={RetinaImg.Mode.ContentIsMask} />
        )}
        <div className="message">{messageContent}</div>
      </div>
    );
  }
}

function EmptyInboxState() {
  return (
    <div className="inbox-zero">
      <RetinaImg name="inbox-zero-plain.png" mode={RetinaImg.Mode.ContentPreserve} />
      <div className="message">{localized('Hooray! You’re done.')}</div>
    </div>
  );
}

class EmptyListState extends React.Component<
  { visible: boolean },
  { active: boolean; syncing: boolean }
> {
  static displayName = 'EmptyListState';

  _mounted = false;
  _unlisteners = [];

  constructor(props: { visible: boolean }) {
    super(props);
    this.state = Object.assign(
      {
        active: false,
      },
      this._getStateFromStores()
    );
  }

  componentDidMount() {
    this._mounted = true;
    this._unlisteners.push(
      FolderSyncProgressStore.listen(() => this.setState(this._getStateFromStores()), this)
    );
    this._unlisteners.push(
      FocusedPerspectiveStore.listen(() => this.setState(this._getStateFromStores()), this)
    );
    if (this.props.visible && !this.state.active) {
      this.setState({ active: true });
    }
  }

  shouldComponentUpdate(
    nextProps: { visible: boolean },
    nextState: { active: boolean; syncing: boolean }
  ) {
    if (nextProps.visible !== this.props.visible) {
      return true;
    }
    return !_.isEqual(nextState, this.state);
  }

  componentWillUnmount() {
    this._mounted = false;
    for (const unlisten of Array.from(this._unlisteners)) {
      unlisten();
    }
  }

  componentDidUpdate(prevProps: { visible: boolean }) {
    if (this.props.visible && !this.state.active) {
      this.setState({ active: true });
    } else if (prevProps.visible && !this.props.visible) {
      this.setState({ active: false });
    }
  }

  render() {
    if (!this.props.visible) {
      return <span />;
    }
    let ContentComponent: React.ComponentType<any> = EmptyPerspectiveState;
    const current = FocusedPerspectiveStore.current();

    let messageContent = current.emptyMessage();
    if (this.state.syncing) {
      messageContent = <SyncingListState empty />;
    } else if (current.isInbox()) {
      ContentComponent = EmptyInboxState;
    }

    const classes = classNames({
      'empty-state': true,
      active: this.state.active,
    });

    return (
      <div className={classes}>
        <ContentComponent perspective={current} messageContent={messageContent} />
      </div>
    );
  }

  _getStateFromStores() {
    return { syncing: FocusedPerspectiveStore.current().hasSyncingCategories() };
  }
}

export default EmptyListState;
