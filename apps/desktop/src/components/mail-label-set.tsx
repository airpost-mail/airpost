import React from 'react';
import FocusedPerspectiveStore from '../data/stores/focused-perspective';
import CategoryStore from '../data/stores/category';
import { MessageStore } from '../data/stores/message';
import { AccountStore } from '../data/stores/account';
import { MailLabel } from './mail-label';
import * as Actions from '../data/actions';
import { ChangeLabelsTask } from '../data/tasks/change-labels';
import { InjectedComponentSet } from './injected/component-set';
import { Thread } from 'mailspring-exports';

const LabelComponentCache = {};

type MailLabelSetProps = {
  thread: Thread;
  messages?: any[];
  includeCurrentCategories?: boolean;
  removable?: boolean;
};

export default class MailLabelSet extends React.Component<MailLabelSetProps> {
  static displayName = 'MailLabelSet';

  _onRemoveLabel(label) {
    const task = new ChangeLabelsTask({
      source: 'Label Remove Icon',
      threads: [this.props.thread],
      labelsToAdd: [],
      labelsToRemove: [label],
    });
    Actions.queueTask(task);
  }

  render() {
    const { thread, messages, includeCurrentCategories } = this.props;
    const account = AccountStore.accountForId(thread.accountId);
    const labels = [];

    if (account && account.usesLabels()) {
      const hidden = CategoryStore.hiddenCategories(thread.accountId);
      let current = FocusedPerspectiveStore.current().categories();

      if (includeCurrentCategories || !current) {
        current = [];
      }

      const ignoredIds = [...hidden, ...current].map((l) => l.id);
      const ignoredNames = MessageStore.FolderNamesHiddenByDefault;

      for (const label of thread.sortedCategories()) {
        const labelExists = CategoryStore.byId(thread.accountId, label.id);
        if (ignoredNames.includes(label.name) || ignoredIds.includes(label.id) || !labelExists) {
          continue;
        }

        if (this.props.removable) {
          labels.push(
            <MailLabel label={label} key={label.id} onRemove={() => this._onRemoveLabel(label)} />
          );
        } else {
          if (LabelComponentCache[label.id] === undefined) {
            LabelComponentCache[label.id] = <MailLabel label={label} key={label.id} />;
          }
          labels.push(LabelComponentCache[label.id]);
        }
      }
    }
    return (
      <InjectedComponentSet
        inline
        containersRequired={false}
        matching={{ role: 'Thread:MailLabel' }}
        className="thread-injected-mail-labels"
        exposedProps={{ thread, messages }}
      >
        {labels}
      </InjectedComponentSet>
    );
  }
}
