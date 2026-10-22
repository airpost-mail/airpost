import React from 'react';
import { localized } from '../../intl/localize';
import { ISendAction, SendActionsStore } from '../../data/stores/send-actions';
import * as Actions from '../../data/actions';
import { Message } from '../../data/models/message';
import { Menu } from '../../components/menu';
import { RetinaImg } from '../../components/retina-img';
import { ButtonDropdown } from '../../components/button-dropdown';

interface SendActionButtonProps {
  tabIndex: number;
  style: any;
  draft: Message;
  isValidDraft: () => boolean;
}

interface SendActionButtonState {
  sendActions: ISendAction[];
}

export class SendActionButton extends React.Component<
  SendActionButtonProps,
  SendActionButtonState
> {
  static displayName = 'SendActionButton';

  static containerRequired = false;

  _unlisteners: Array<() => void> = [];
  _composedComponent: any;

  constructor(props: SendActionButtonProps) {
    super(props);
    this.state = {
      sendActions: SendActionsStore.orderedSendActionsForDraft(props.draft),
    };
  }

  componentDidMount() {
    this._unlisteners.push(
      SendActionsStore.listen(() => {
        this.setState({
          sendActions: SendActionsStore.orderedSendActionsForDraft(this.props.draft),
        });
      })
    );
  }

  componentDidUpdate(prevProps: SendActionButtonProps) {
    if (prevProps.draft !== this.props.draft) {
      this.setState({
        sendActions: SendActionsStore.orderedSendActionsForDraft(this.props.draft),
      });
    }
  }

  componentWillUnmount() {
    for (const unlisten of this._unlisteners) {
      unlisten();
    }
    this._unlisteners = [];
  }

  /* This component is re-rendered constantly because `draft` changes in random ways.
  We only use the draft prop when you click send, so update with more discretion. */
  shouldComponentUpdate(nextProps: SendActionButtonProps, nextState: SendActionButtonState) {
    return (
      nextState.sendActions.map((a) => a.configKey).join(',') !==
      this.state.sendActions.map((a) => a.configKey).join(',')
    );
  }

  primarySend() {
    this._onPrimaryClick();
  }

  _onPrimaryClick = () => {
    this._onSendWithAction(this.state.sendActions[0]);
  };

  _onSendWithAction = (sendAction: ISendAction) => {
    if (this.props.isValidDraft()) {
      if (AppEnv.config.get('core.sending.sounds')) {
        Actions.playSound('hit-send');
      }
      Actions.sendDraft(this.props.draft.headerMessageId, { actionKey: sendAction.configKey });
    }
  };

  _renderSendActionItem = ({ iconUrl }: { iconUrl?: string | null }) => {
    let plusHTML: React.ReactChild = '';
    let additionalImg: React.ReactChild | null = null;

    if (iconUrl) {
      plusHTML = <span>&nbsp;+&nbsp;</span>;
      additionalImg = <RetinaImg url={iconUrl} mode={RetinaImg.Mode.ContentIsMask} />;
    }

    return (
      <span>
        <RetinaImg
          name="icon-composer-send.png"
          mode={RetinaImg.Mode.ContentIsMask}
          aria-hidden="true"
        />
        <span className="text">
          {localized(`Send`)}
          {plusHTML}
        </span>
        {additionalImg}
      </span>
    );
  };

  render() {
    return (
      <ButtonDropdown
        className={'btn-send btn-emphasis btn-text'}
        style={{ order: -100 }}
        primaryItem={this._renderSendActionItem(this.state.sendActions[0])}
        primaryTitle={this.state.sendActions[0].title}
        primaryClick={this._onPrimaryClick}
        closeOnMenuClick
        menu={
          <Menu
            items={this.state.sendActions.slice(1)}
            itemKey={(actionConfig) => actionConfig.configKey}
            itemContent={this._renderSendActionItem}
            onSelect={this._onSendWithAction}
          />
        }
      />
    );
  }
}
