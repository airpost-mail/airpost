import React from 'react';
import { localized } from '../../intl/localize';
import { IconButton } from '../../components/kit/icon-button';
import EmailFrameStylesStore from './email-frame-styles-store';
import {
  EMAIL_RENDER_MODE_KEY,
  OriginalColors,
  darkeningApplies,
  themeBackgroundAt,
} from './email-render-mode';

interface SubjectLineIconsProps {
  canCollapse: boolean;
  hasCollapsedItems: boolean;
  messageIds: string[];

  onPrint: () => void;
  onPopIn: () => void;
  onPopOut: () => void;
  onToggleAllExpanded: () => void;
}

interface OriginalColorsToggleState {
  applies: boolean;
  original: boolean;
}

// Switches the thread's messages between darkened and original colours. Hidden unless emails
// are being darkened, which depends on the theme around it.
class OriginalColorsToggle extends React.Component<
  { messageIds: string[] },
  OriginalColorsToggleState
> {
  _el: HTMLButtonElement | null = null;
  _disposables: Array<() => void> = [];

  state = { applies: false, original: false };

  componentDidMount() {
    const configDisposable = AppEnv.config.onDidChange(EMAIL_RENDER_MODE_KEY, this._update);
    this._disposables = [
      () => configDisposable.dispose(),
      EmailFrameStylesStore.listen(this._update),
      OriginalColors.listen(this._update),
    ];
    this._update();
  }

  componentDidUpdate(prevProps: { messageIds: string[] }) {
    if (prevProps.messageIds.join() !== this.props.messageIds.join()) this._update();
  }

  componentWillUnmount() {
    this._disposables.forEach((dispose) => dispose());
  }

  _update = () => {
    if (!this._el) return;
    const { messageIds } = this.props;
    this.setState({
      applies: darkeningApplies(themeBackgroundAt(this._el)),
      original:
        messageIds.length > 0 && messageIds.every((id) => OriginalColors.isShowingOriginal(id)),
    });
  };

  _onToggle = () => {
    OriginalColors.setShowingOriginal(this.props.messageIds, !this.state.original);
  };

  render() {
    const { applies, original } = this.state;
    const title = original ? localized('Show Dark Colors') : localized('Show Original Colors');

    return (
      <IconButton
        className="original-colors-toggle"
        size="sm"
        icon={original ? 'toolbar-bulb-off.png' : 'toolbar-bulb-on.png'}
        label={title}
        style={applies ? undefined : { display: 'none' }}
        ref={(el) => {
          this._el = el;
        }}
        onClick={this._onToggle}
      />
    );
  }
}

export const SubjectLineIcons: React.FunctionComponent<SubjectLineIconsProps> = (props) => (
  <div className="message-icons-wrap">
    {props.canCollapse && (
      <IconButton
        size="sm"
        icon={props.hasCollapsedItems ? 'expand.png' : 'collapse.png'}
        label={props.hasCollapsedItems ? localized('Expand All') : localized('Collapse All')}
        onClick={props.onToggleAllExpanded}
      />
    )}
    <IconButton
      size="sm"
      icon="print.png"
      label={localized('Print Thread')}
      onClick={props.onPrint}
    />
    <OriginalColorsToggle messageIds={props.messageIds} />
    {AppEnv.isThreadWindow() ? (
      <IconButton
        size="sm"
        icon="thread-popin.png"
        label={localized('Pop thread in')}
        onClick={props.onPopIn}
      />
    ) : (
      <IconButton
        size="sm"
        icon="thread-popout.png"
        label={localized('Popout thread')}
        onClick={props.onPopOut}
      />
    )}
  </div>
);
