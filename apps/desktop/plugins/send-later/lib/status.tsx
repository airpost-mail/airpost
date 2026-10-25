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
import { cancelSend } from '../../../src/engine/client/drafts';
import { PLUGIN_ID } from './constants';

const { DATE_FORMAT_SHORT } = DateUtils;

// An expiration closer than this is an undo-send window, shown as a countdown.
const COUNTDOWN_MS = 60 * 1000;

interface SendLaterStatusProps {
  draft: Message;
}
interface SendLaterStatusState {
  // The draft's SendDraftTask while it is queued or running.
  task: SendDraftTask | undefined;
}

function isActive(task: SendDraftTask) {
  return task.status !== 'complete' && task.status !== 'cancelled';
}

export default class SendLaterStatus extends Component<SendLaterStatusProps, SendLaterStatusState> {
  static displayName = 'SendLaterStatus';

  _unlisten?: () => void;
  _ticker: ReturnType<typeof setInterval> | null = null;

  constructor(props: SendLaterStatusProps) {
    super(props);
    this.state = this.getStateFromStores(props);
  }

  componentDidMount() {
    this._unlisten = TaskQueue.listen(() => {
      this.setState(this.getStateFromStores(this.props));
    });
    this._syncTicker();
  }

  componentDidUpdate(prevProps: SendLaterStatusProps) {
    if (prevProps.draft !== this.props.draft) {
      this.setState(this.getStateFromStores(this.props));
    }
    this._syncTicker();
  }

  componentWillUnmount() {
    if (this._unlisten) {
      this._unlisten();
    }
    this._stopTicker();
  }

  getStateFromStores({ draft }: SendLaterStatusProps) {
    const tasks = TaskQueue.findTasks(SendDraftTask, {
      headerMessageId: draft.headerMessageId,
    }) as SendDraftTask[];
    return { task: tasks.find(isActive) };
  }

  _expiration(): Date | null {
    const metadata = this.props.draft.metadataForPluginId(PLUGIN_ID);
    return metadata && metadata.expiration ? metadata.expiration : null;
  }

  _syncTicker() {
    const expiration = this._expiration();
    const left = expiration ? expiration.getTime() - Date.now() : 0;
    const counting = !this.state.task && left > 0 && left <= COUNTDOWN_MS;
    if (counting && !this._ticker) {
      this._ticker = setInterval(() => this.forceUpdate(), 1000);
    } else if (!counting) {
      this._stopTicker();
    }
  }

  _stopTicker() {
    if (this._ticker) {
      clearInterval(this._ticker);
      this._ticker = null;
    }
  }

  onClearSendLater = () => {
    Actions.queueTask(
      SyncbackMetadataTask.forSaving({
        model: this.props.draft,
        pluginId: PLUGIN_ID,
        value: { expiration: null },
      })
    );
  };

  // The engine decides the cancel against its executor, so a cancel can't land after the send
  // was queued.
  onCancelSend = () => {
    const { accountId, headerMessageId } = this.props.draft;
    cancelSend({ accountId, headerMessageId }).then(
      (cancelled) => {
        if (!cancelled) AppEnv.showErrorDialog(localized('The message was already sent.'));
      },
      (err) =>
        AppEnv.showErrorDialog(localized('Sorry, the send could not be undone. %@', err.message))
    );
  };

  _renderSending() {
    return (
      <div className="send-later-status send-later-sending">
        <span className="time">{localized('Sending…')}</span>
      </div>
    );
  }

  render() {
    const { task } = this.state;
    if (task && task.offlineSince) {
      // The engine keeps the send queued until the account can reach its server again.
      return (
        <div className="send-later-status send-later-queued">
          <span className="time">{localized('Queued, sends when online')}</span>
        </div>
      );
    }
    if (task) {
      return this._renderSending();
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
            onClick={this.onClearSendLater}
          />
        </div>
      );
    }

    const expiration = this._expiration();
    if (!expiration) {
      return <span />;
    }
    const left = expiration.getTime() - Date.now();
    if (left <= 0) {
      // Due, and about to be queued by the engine's executor (or this window's send-later).
      return this._renderSending();
    }

    const countdown = left <= COUNTDOWN_MS;
    const label = countdown
      ? localized('Sending in %@s', Math.ceil(left / 1000))
      : localized('Scheduled for %@', DateUtils.format(moment(expiration), DATE_FORMAT_SHORT));

    return (
      <div className="send-later-status">
        <span className="time">{label}</span>
        <IconButton
          icon="image-cancel-button.png"
          mode={RetinaImg.Mode.ContentPreserve}
          label={countdown ? localized('Cancel send') : localized('Cancel Send Later')}
          onClick={this.onCancelSend}
        />
      </div>
    );
  }
}
