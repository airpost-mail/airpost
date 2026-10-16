import React from 'react';
import { RetinaImg } from 'mailspring-component-kit';
import { localized } from 'mailspring-exports';
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
  _el: HTMLDivElement | null = null;
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
      <div
        role="button"
        tabIndex={0}
        aria-label={title}
        title={title}
        style={applies ? undefined : { display: 'none' }}
        ref={(el) => {
          this._el = el;
        }}
        onClick={this._onToggle}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            this._onToggle();
          }
        }}
      >
        <RetinaImg
          name={original ? 'toolbar-bulb-off.png' : 'toolbar-bulb-on.png'}
          mode={RetinaImg.Mode.ContentIsMask}
          style={{ width: 16, height: 16 }}
          aria-hidden="true"
        />
      </div>
    );
  }
}

export const SubjectLineIcons: React.FunctionComponent<SubjectLineIconsProps> = (props) => (
  <div className="message-icons-wrap">
    {props.canCollapse && (
      <div
        role="button"
        tabIndex={0}
        aria-label={props.hasCollapsedItems ? localized('Expand All') : localized('Collapse All')}
        title={props.hasCollapsedItems ? localized('Expand All') : localized('Collapse All')}
        onClick={props.onToggleAllExpanded}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            props.onToggleAllExpanded();
          }
        }}
      >
        <RetinaImg
          name={props.hasCollapsedItems ? 'expand.png' : 'collapse.png'}
          mode={RetinaImg.Mode.ContentIsMask}
          aria-hidden="true"
        />
      </div>
    )}
    <div
      role="button"
      tabIndex={0}
      aria-label={localized('Print Thread')}
      title={localized('Print Thread')}
      onClick={props.onPrint}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          props.onPrint();
        }
      }}
    >
      <RetinaImg name="print.png" mode={RetinaImg.Mode.ContentIsMask} aria-hidden="true" />
    </div>
    <OriginalColorsToggle messageIds={props.messageIds} />
    {AppEnv.isThreadWindow() ? (
      <div
        role="button"
        tabIndex={0}
        aria-label={localized('Pop thread in')}
        title={localized('Pop thread in')}
        onClick={props.onPopIn}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            props.onPopIn();
          }
        }}
      >
        <RetinaImg name="thread-popin.png" mode={RetinaImg.Mode.ContentIsMask} aria-hidden="true" />
      </div>
    ) : (
      <div
        role="button"
        tabIndex={0}
        aria-label={localized('Popout thread')}
        title={localized('Popout thread')}
        onClick={props.onPopOut}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            props.onPopOut();
          }
        }}
      >
        <RetinaImg
          name="thread-popout.png"
          mode={RetinaImg.Mode.ContentIsMask}
          aria-hidden="true"
        />
      </div>
    )}
  </div>
);
