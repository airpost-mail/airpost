/* eslint react/no-danger: 0 */
import _ from 'underscore';
import React from 'react';
import ReactDOM from 'react-dom';

import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import WorkspaceStore from '../../data/stores/workspace';
import DOMUtils from '../../utils/dom';
import BaseStore from '../../data/stores/base';
import { Disposable } from 'event-kit';

const TipsBackgroundEl = document.createElement('tutorial-tip-background');

const TipsContainerEl = document.createElement('div');
TipsContainerEl.classList.add('tooltips-container');
document.body.insertBefore(TipsContainerEl, document.body.children[0]);

class TipsStoreCls extends BaseStore {
  _tipKeys: string[] = [];

  isTipVisible(key: string) {
    const seen: string[] = AppEnv.config.get('core.tutorial.seen') || [];
    return this._tipKeys.find((t) => !seen.includes(t)) === key;
  }

  hasSeenTip(key: string) {
    return (AppEnv.config.get('core.tutorial.seen') || []).includes(key);
  }

  // Actions: Since this is a private store just inside this file, we call
  // these methods directly for now.

  mountedTip = (key: string) => {
    if (!this._tipKeys.includes(key)) {
      this._tipKeys.push(key);
    }
    this.trigger();
  };

  seenTip = (key: string) => {
    this._tipKeys = this._tipKeys.filter((t) => t !== key);
    AppEnv.config.pushAtKeyPath('core.tutorial.seen', key);
    this.trigger();
  };

  unmountedTip = (key: string) => {
    this._tipKeys = this._tipKeys.filter((t) => t !== key);
    this.trigger();
  };
}

const TipsStore = new TipsStoreCls();

interface TipPopoverContentsProps {
  title: string;
  tipKey: string;
  instructions: React.ReactNode;
  onDismissed: () => void;
}

interface TipConfig {
  title: string;
  // A string is rendered as HTML
  instructions: React.ReactNode;
}

type TipTarget<P> = React.ComponentType<P> & {
  displayName?: string;
  containerRequired?: boolean;
  containerStyles?: React.CSSProperties;
};

class TipPopoverContents extends React.Component<TipPopoverContentsProps> {
  componentDidMount() {
    if (TipsBackgroundEl.parentNode === null) {
      document.body.appendChild(TipsBackgroundEl);
    }
    window.requestAnimationFrame(() => {
      TipsBackgroundEl.classList.add('visible');
    });
  }

  componentWillUnmount() {
    TipsBackgroundEl.classList.remove('visible');
    if (this.props.onDismissed) {
      this.props.onDismissed();
    }
  }

  onDone = () => {
    TipsStore.seenTip(this.props.tipKey);
    Actions.closePopover();
  };

  render() {
    let content = null;

    if (typeof this.props.instructions === 'string') {
      content = <p dangerouslySetInnerHTML={{ __html: this.props.instructions }} />;
    } else {
      content = <p>{this.props.instructions}</p>;
    }

    return (
      <div style={{ width: 250, padding: 20, paddingTop: 0 }}>
        <h2>{this.props.title}</h2>
        {content}
        <button className="btn" onClick={this.onDone}>
          {localized('Got it!')}
        </button>
      </div>
    );
  }
}

// Returns `any`, as it did implicitly before: callers subclass the result (mode-switch) and
// register it, and it is either the wrapper class or the original component.
export default function HasTutorialTip<P extends object>(
  ComposedComponent: TipTarget<P>,
  TipConfig: TipConfig
): any {
  const displayName = ComposedComponent.displayName;

  if (!displayName) {
    throw new Error('To use the HasTutorialTip decorator, your component must have a displayName.');
  }
  const TipKey: string = displayName;
  if (TipsStore.hasSeenTip(TipKey)) {
    return ComposedComponent;
  }

  return class extends React.Component<P, { visible: boolean }> {
    static displayName = ComposedComponent.displayName;
    static containerRequired = ComposedComponent.containerRequired;
    static containerStyles = ComposedComponent.containerStyles;

    // Both are set in componentDidMount; everything else that reads them runs after mount.
    tipNode!: HTMLElement;
    tipAnchor!: Element;

    _workspaceTimer?: NodeJS.Timeout;
    _themesTimer?: NodeJS.Timeout;
    _mounted = false;
    _disposables: Disposable[] = [];
    _unlisteners: Array<() => void> = [];

    constructor(props: P) {
      super(props);
      this.state = { visible: false };
    }

    componentDidMount() {
      this._mounted = true;
      TipsStore.mountedTip(TipKey);

      // Note: We use a _mounted flag because of these setTimeout calls
      this._unlisteners = [
        TipsStore.listen(this._onTooltipStateChanged),
        WorkspaceStore.listen(() => {
          this._workspaceTimer = setTimeout(this._onTooltipStateChanged, 0);
        }),
      ];
      this._disposables = [
        AppEnv.themes.onDidChangeActiveThemes(() => {
          this._themesTimer = setTimeout(this._onRecomputeTooltipPosition, 0);
        }),
      ];
      window.addEventListener('resize', this._onRecomputeTooltipPosition);

      // unfortunately, we can't render() a container around ComposedComponent
      // without modifying the DOM tree and messing with things like flexbox.
      // Instead, we leave render() unchanged and attach the bubble and hover
      // listeners to the DOM manually.
      const el = ReactDOM.findDOMNode(this) as HTMLElement;

      this.tipNode = document.createElement('div');
      this.tipNode.classList.add('tutorial-tip');

      this.tipAnchor = el.closest('[data-tooltips-anchor]') || document.body;
      // Tooltip anchors carry a .tooltips-container; document.body gets one at the top of this file.
      this.tipAnchor.querySelector('.tooltips-container')!.appendChild(this.tipNode);

      el.addEventListener('mouseover', this._onMouseOver);
      this._onTooltipStateChanged();
    }

    componentDidUpdate() {
      if (this.state.visible) {
        this._onRecomputeTooltipPosition();
      }
    }

    componentWillUnmount() {
      this._mounted = false;
      this._unlisteners.forEach((unlisten) => unlisten());
      this._disposables.forEach((disposable) => disposable.dispose());

      window.removeEventListener('resize', this._onRecomputeTooltipPosition);
      // Appended in componentDidMount
      this.tipNode.parentNode!.removeChild(this.tipNode);
      clearTimeout(this._workspaceTimer);
      clearTimeout(this._themesTimer);

      TipsStore.unmountedTip(TipKey);
    }

    _containingSheetIsVisible = (el: Element) => {
      const sheetEl =
        el.closest<HTMLElement>('.sheet') || el.closest<HTMLElement>('.sheet-toolbar-container');
      if (!sheetEl) {
        return true;
      }
      return sheetEl.dataset.id === WorkspaceStore.topSheet().id;
    };

    _isVisible = () => {
      if (!this._mounted) return false;
      const el = ReactDOM.findDOMNode(this);
      if (!(el instanceof Element)) return false;
      return (
        TipsStore.isTipVisible(TipKey) &&
        this._containingSheetIsVisible(el) &&
        DOMUtils.nodeIsVisible(el)
      );
    };

    _onTooltipStateChanged = () => {
      if (!this._mounted) return;
      const visible = this._isVisible();
      if (this.state.visible !== visible) {
        this.setState({ visible });
        if (visible) {
          this.tipNode.classList.add('visible');
          this._onRecomputeTooltipPosition();
        } else {
          this.tipNode.classList.remove('visible');
        }
      }
    };

    _onMouseOver = () => {
      if (!this._mounted) return;
      if (!this.state.visible) return;

      const el = ReactDOM.findDOMNode(this);
      if (!el) return;
      el.removeEventListener('mouseover', this._onMouseOver);

      const tipRect = this.tipNode.getBoundingClientRect();
      const tipFocusCircleRadius = 64;
      const rect = (ReactDOM.findDOMNode(this) as HTMLElement).getBoundingClientRect();
      if (rect.width > 250 || rect.height > 250) {
        // Focus the gradient on the center of the pusling dot because the element is too large
        const rectCX = Math.round(tipRect.left + tipRect.width / 2 - tipFocusCircleRadius);
        const rectCY = Math.round(tipRect.top + tipRect.height / 2 - tipFocusCircleRadius);
        TipsBackgroundEl.style.webkitMaskPosition = `0 0, ${rectCX}px ${rectCY}px`;
      } else {
        // Focus the gradient on the center of the element being explained
        const rectCX = Math.round(rect.left + rect.width / 2 - tipFocusCircleRadius);
        const rectCY = Math.round(rect.top + rect.height / 2 - tipFocusCircleRadius);
        TipsBackgroundEl.style.webkitMaskPosition = `0 0, ${rectCX}px ${rectCY}px`;
      }

      Actions.openPopover(
        <TipPopoverContents
          tipKey={TipKey}
          title={TipConfig.title}
          instructions={TipConfig.instructions}
          onDismissed={() => {
            el.addEventListener('mouseover', this._onMouseOver);
          }}
        />,
        {
          originRect: tipRect,
          direction: 'down',
          fallbackDirection: 'up',
        }
      );
    };

    _onRecomputeTooltipPosition = () => {
      if (!this._mounted) return;
      const el = ReactDOM.findDOMNode(this) as HTMLElement;
      let settled = 0;
      let last: { left?: number; top?: number } = {};
      const attempt = () => {
        const { left, top } = el.getBoundingClientRect();
        const anchorRect = this.tipAnchor.getBoundingClientRect();

        this.tipNode.style.left = `${left - anchorRect.left + 5}px`;
        this.tipNode.style.top = `${Math.max(top - anchorRect.top + 5, 10)}px`;

        if (!_.isEqual(last, { left, top })) {
          settled = 0;
          last = { left, top };
        }
        settled += 1;
        if (settled < 5) {
          window.requestAnimationFrame(attempt);
        }
      };
      attempt();
    };

    render() {
      return <ComposedComponent {...this.props} />;
    }
  };
}
