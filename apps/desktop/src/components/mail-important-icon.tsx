import React from 'react';
import _ from 'underscore';
import classNames from 'classnames';
import { localized } from '../intl/localize';
import * as Actions from '../data/actions';
import { ChangeLabelsTask } from '../data/tasks/change-labels';
import CategoryStore from '../data/stores/category';
import FocusedPerspectiveStore from '../data/stores/focused-perspective';
import { AccountStore } from '../data/stores/account';
import { Thread } from '../data/models/thread';
import { Category } from '../data/models/category';
import { Label } from '../data/models/label';
import { Disposable } from 'rx-core';

const ShowImportantKey = 'core.workspace.showImportant';

type MailImportantIconProps = {
  thread: Thread;
  showIfAvailableForAnyAccount?: boolean;
};
type MailImportantIconState = {
  visible: boolean;
  category: Category | null;
  isImportant: boolean;
};

class MailImportantIcon extends React.Component<MailImportantIconProps, MailImportantIconState> {
  static displayName = 'MailImportantIcon';

  unsubscribe?: () => void;
  subscription?: Disposable;

  constructor(props: MailImportantIconProps) {
    super(props);
    this.state = this.getState();
  }

  getState = (props = this.props) => {
    let category: Category | null = null;
    let visible = false;

    if (props.showIfAvailableForAnyAccount) {
      const perspective = FocusedPerspectiveStore.current();
      for (const accountId of perspective.accountIds) {
        const account = AccountStore.accountForId(accountId);
        const accountImportant = CategoryStore.getCategoryByRole(account ?? null, 'important');
        if (accountImportant) {
          visible = true;
        }
        if (accountId === props.thread.accountId) {
          category = accountImportant;
        }
        if (visible && category) {
          break;
        }
      }
    } else {
      category = CategoryStore.getCategoryByRole(props.thread.accountId, 'important');
      visible = category != null;
    }

    const isImportant =
      !!category && props.thread.labels?.find((x) => x.id === category.id) != null;

    return { visible, category, isImportant };
  };

  componentDidMount() {
    this.unsubscribe = FocusedPerspectiveStore.listen(() => {
      this.setState(this.getState());
    });
    this.subscription = AppEnv.config.onDidChange(ShowImportantKey, () => {
      this.setState(this.getState());
    });
  }

  componentDidUpdate(prevProps: MailImportantIconProps) {
    if (
      prevProps.thread !== this.props.thread ||
      prevProps.showIfAvailableForAnyAccount !== this.props.showIfAvailableForAnyAccount
    ) {
      this.setState(this.getState());
    }
  }

  componentWillUnmount() {
    if (typeof this.unsubscribe === 'function') {
      this.unsubscribe();
    }
    if (this.subscription) {
      this.subscription.dispose();
    }
  }

  // A new thread prop must get through so componentDidUpdate can derive state from it.
  shouldComponentUpdate(nextProps: MailImportantIconProps, nextState: MailImportantIconState) {
    return (
      nextProps.thread !== this.props.thread ||
      nextProps.showIfAvailableForAnyAccount !== this.props.showIfAvailableForAnyAccount ||
      !_.isEqual(nextState, this.state)
    );
  }

  render() {
    let title;
    if (!this.state.visible) {
      return false;
    }

    const classes = classNames({
      'mail-important-icon': true,
      enabled: this.state.category != null,
      active: this.state.isImportant,
    });

    if (!this.state.category) {
      title = localized('No important folder / label');
    } else if (this.state.isImportant) {
      title = localized('Mark as Not Important');
    } else {
      title = localized('Mark as Important');
    }

    return <div className={classes} title={title} onClick={this._onToggleImportant} />;
  }

  _onToggleImportant = (event: React.MouseEvent) => {
    const { category } = this.state;

    if (category) {
      const isImportant = this.props.thread.labels?.find((x) => x.id === category.id) != null;

      if (!isImportant) {
        Actions.queueTask(
          new ChangeLabelsTask({
            labelsToAdd: [category],
            labelsToRemove: [],
            threads: [this.props.thread],
            source: 'Important Icon',
          })
        );
      } else {
        Actions.queueTask(
          new ChangeLabelsTask({
            labelsToAdd: [],
            labelsToRemove: [category],
            threads: [this.props.thread],
            source: 'Important Icon',
          })
        );
      }
    }

    // Don't trigger the thread row click
    event.stopPropagation();
  };
}

export default MailImportantIcon;
