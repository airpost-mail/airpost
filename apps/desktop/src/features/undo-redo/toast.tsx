import React from 'react';
import { localized } from '../../intl/localize';
import UndoRedoStore, { UndoBlock } from '../../data/stores/undo-redo';
import { SyncbackMetadataTask } from '../../data/tasks/syncback-metadata';
import DatabaseStore from '../../data/database/store';
import { Message } from '../../data/models/message';
import * as Actions from '../../data/actions';
import { RetinaImg } from '../../components/retina-img';
import { Button } from '../../components/kit/button';
import { CSSTransition, TransitionGroup } from 'react-transition-group';
import { PLUGIN_ID } from '../../../plugins/send-later/lib/constants';

function isUndoSend(block: UndoBlock) {
  return (
    block.tasks.length === 1 &&
    block.tasks[0] instanceof SyncbackMetadataTask &&
    (block.tasks[0].value as any).isUndoSend
  );
}

// Moves the undo window to now. The rest of the value stays: with the draft pipeline the
// engine sends the bodies draft.send kept only while `isUndoSend` is set, and `actionKey`
// (send and archive) and `clientFinalize` still decide what happens after.
export async function sendMessageNow(block: UndoBlock) {
  if (isUndoSend(block)) {
    const message = await DatabaseStore.find<Message>(
        Message,
        (block.tasks[0] as SyncbackMetadataTask).modelId
      ),
      newExpiry = Math.floor(Date.now() / 1000);

    // Message may be null if the sync engine already processed/sent it before
    // the user clicked "Send now". In that case there's nothing to do.
    if (!message) {
      return true;
    }

    Actions.queueTask(
      SyncbackMetadataTask.forSaving({
        model: message,
        pluginId: PLUGIN_ID,
        value: {
          ...(block.tasks[0] as SyncbackMetadataTask).value,
          expiration: newExpiry,
        },
      })
    );

    (block.tasks[0] as SyncbackMetadataTask).value.expiration = newExpiry;

    return true;
  }

  return false;
}

function getUndoSendExpiration(block: UndoBlock) {
  return (block.tasks[0] as SyncbackMetadataTask).value.expiration * 1000;
}

function getDisplayDuration(block: UndoBlock) {
  return isUndoSend(block) ? Math.max(400, getUndoSendExpiration(block) - Date.now()) : 3000;
}

interface ToastContentProps {
  block: UndoBlock;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}

class Countdown extends React.Component<{ expiration: number }, { x: number }> {
  animationDuration: string;
  _tickStart?: NodeJS.Timeout;
  _tick?: NodeJS.Timeout;

  constructor(props: { expiration: number }) {
    super(props);
    this.animationDuration = `${props.expiration - Date.now()}ms`;
    this.state = { x: 0 };
  }

  componentDidUpdate(prevProps: { expiration: number }) {
    if (prevProps.expiration !== this.props.expiration) {
      this.animationDuration = `${this.props.expiration - Date.now()}ms`;
    }
  }

  componentDidMount() {
    this._tickStart = setTimeout(() => {
      this.setState({ x: this.state.x + 1 });
      this._tick = setInterval(() => {
        this.setState({ x: this.state.x + 1 });
      }, 1000);
    }, this.props.expiration % 1000);
  }

  componentWillUnmount() {
    clearTimeout(this._tickStart);
    clearInterval(this._tick);
  }

  render() {
    // subtract a few ms so we never round up to start time + 1 by accident
    const diff = Math.min(
      Math.max(0, this.props.expiration - Date.now()),
      AppEnv.config.get('core.sending.undoSend')
    );

    return (
      <div className="countdown">
        <div className="countdown-number">{Math.ceil(diff / 1000)}</div>
        {diff > 0 && (
          <svg>
            <circle r="14" cx="15" cy="15" style={{ animationDuration: this.animationDuration }} />
          </svg>
        )}
      </div>
    );
  }
}

const UndoSendContent = ({ block, onMouseEnter, onMouseLeave }: ToastContentProps) => {
  return (
    <div className="content" onMouseEnter={onMouseEnter} onMouseLeave={onMouseLeave}>
      <Countdown expiration={getUndoSendExpiration(block)} />
      <div className="message">{localized('Sending soon...')}</div>
      <Button
        variant="ghost"
        className="action"
        onClick={async () => {
          (await sendMessageNow(block)) && onMouseLeave();
        }}
      >
        <RetinaImg name="icon-composer-send.png" mode={RetinaImg.Mode.ContentIsMask} />
        <span className="send-action-text">{localized('Send now instead')}</span>
      </Button>
      <Button
        variant="ghost"
        className="action"
        onClick={() => AppEnv.commands.dispatch('core:undo')}
      >
        <RetinaImg name="undo-icon@2x.png" mode={RetinaImg.Mode.ContentIsMask} />
        <span className="undo-action-text">{localized('Undo')}</span>
      </Button>
    </div>
  );
};

const BasicContent = ({ block, onMouseEnter, onMouseLeave }: ToastContentProps) => {
  return (
    <div className="content" onMouseEnter={onMouseEnter} onMouseLeave={onMouseLeave}>
      <div className="message">{block.description}</div>
      <Button
        variant="ghost"
        className="action"
        onClick={() => AppEnv.commands.dispatch('core:undo')}
      >
        <RetinaImg name="undo-icon@2x.png" mode={RetinaImg.Mode.ContentIsMask} />
        <span className="undo-action-text">{localized('Undo')}</span>
      </Button>
    </div>
  );
};

export default class UndoRedoToast extends React.Component<
  Record<string, unknown>,
  { block: UndoBlock | null }
> {
  static displayName = 'UndoRedoToast';
  static containerRequired = false;

  _timeout: NodeJS.Timeout | null;
  _unlisten?: () => void;

  constructor(props: Record<string, unknown>) {
    super(props);

    this._timeout = null;
    this._unlisten = undefined;

    // Note: we explicitly do /not/ set initial state to the state of
    // the UndoRedoStore here because "getMostRecent" might be more
    // than 3000ms old.
    this.state = {
      block: null,
    };
  }

  componentDidMount() {
    this._unlisten = UndoRedoStore.listen(() => {
      this.setState({
        block: UndoRedoStore.getMostRecent(),
      });
    });
  }

  componentDidUpdate() {
    this._ensureTimeout();
  }

  componentWillUnmount() {
    this._clearTimeout();
    if (this._unlisten) {
      this._unlisten();
    }
  }

  _clearTimeout() {
    if (this._timeout) clearTimeout(this._timeout);
    this._timeout = null;
  }

  _ensureTimeout() {
    this._clearTimeout();

    if (this.state.block) {
      this._timeout = setTimeout(() => {
        this.setState({ block: null });
      }, getDisplayDuration(this.state.block));
    }
  }

  _onMouseEnter = () => {
    this._clearTimeout();
  };

  _onMouseLeave = () => {
    this._ensureTimeout();
  };

  render() {
    const { block } = this.state;
    const Content = block && isUndoSend(block) ? UndoSendContent : BasicContent;

    return (
      <TransitionGroup className="undo-redo-toast" component="div">
        {block ? (
          // Undo blocks carry no id, so every toast shares one key.
          <CSSTransition key="toast" classNames="undo-redo-toast-fade" timeout={150}>
            <Content
              block={block}
              onMouseEnter={this._onMouseEnter}
              onMouseLeave={this._onMouseLeave}
            />
          </CSSTransition>
        ) : null}
      </TransitionGroup>
    );
  }
}
