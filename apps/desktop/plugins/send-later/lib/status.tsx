import React, { Component } from 'react';
import moment from 'moment';
import {
  localized,
  DateUtils,
  Actions,
  Message,
  SyncbackMetadataTask,
  TaskQueue,
  SendDraftTask,
} from 'mailspring-exports';
import { IconButton, RetinaImg } from 'mailspring-component-kit';
import { PLUGIN_ID } from './constants';

const { DATE_FORMAT_SHORT } = DateUtils;

interface SendLaterStatusProps {
  draft: Message;
}
interface SendLaterStatusState {
  task: SendDraftTask | undefined;
}

export default class SendLaterStatus extends Component<SendLaterStatusProps, SendLaterStatusState> {
  static displayName = 'SendLaterStatus';

  _unlisten?: () => void;

  constructor(props: SendLaterStatusProps) {
    super(props);
    this.state = this.getStateFromStores(props);
  }

  componentDidMount() {
    this._unlisten = TaskQueue.listen(() => {
      this.setState(this.getStateFromStores(this.props));
    });
  }

  componentDidUpdate(prevProps: SendLaterStatusProps) {
    if (prevProps.draft !== this.props.draft) {
      this.setState(this.getStateFromStores(this.props));
    }
  }

  componentWillUnmount() {
    if (this._unlisten) {
      this._unlisten();
    }
  }

  onCancelSendLater = () => {
    Actions.queueTask(
      SyncbackMetadataTask.forSaving({
        model: this.props.draft,
        pluginId: PLUGIN_ID,
        value: { expiration: null },
      })
    );
  };

  getStateFromStores({ draft }: SendLaterStatusProps) {
    return {
      task: TaskQueue.findTasks(
        SendDraftTask,
        { headerMessageId: draft.headerMessageId },
        { includeCompleted: true }
      ).pop() as SendDraftTask | undefined,
    };
  }

  render() {
    const { task } = this.state;
    if (task && task.offlineSince && task.status !== 'complete' && task.status !== 'cancelled') {
      // The engine keeps the send queued until the account can reach its server again.
      return (
        <div className="send-later-status send-later-queued">
          <span className="time">{localized('Queued, sends when online')}</span>
        </div>
      );
    }
    const metadata = this.props.draft.metadataForPluginId(PLUGIN_ID);
    if (metadata && !metadata.expiration && metadata.error) {
      // The engine couldn't send it at its time.
      return (
        <div className="send-later-status send-later-error" title={metadata.error}>
          <span className="time">{localized('Not sent: %@', metadata.error)}</span>
          <IconButton
            icon="image-cancel-button.png"
            mode={RetinaImg.Mode.ContentPreserve}
            label={localized('Dismiss')}
            onClick={this.onCancelSendLater}
          />
        </div>
      );
    }
    if (!metadata || !metadata.expiration) {
      return <span />;
    }

    const { expiration } = metadata;

    let label = null;
    if (expiration > new Date(Date.now() + 60 * 1000)) {
      label = localized(
        `Scheduled for %@`,
        DateUtils.format(moment(expiration), DATE_FORMAT_SHORT)
      );
    } else {
      label = localized(`Sending in a few seconds`) + '...';
      if (this.state.task) {
        label = localized(`Sending now`) + '...';
      }
    }

    return (
      <div className="send-later-status">
        <span className="time">{label}</span>
        <IconButton
          icon="image-cancel-button.png"
          mode={RetinaImg.Mode.ContentPreserve}
          label={localized('Cancel Send Later')}
          onClick={this.onCancelSendLater}
        />
      </div>
    );
  }
}
