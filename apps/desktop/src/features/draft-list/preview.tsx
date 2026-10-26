import React from 'react';
import { localized } from '../../intl/localize';
import DraftStore from '../../data/drafts/store';
import ComponentRegistry from '../../packages/component-registry';
import { ScrollRegion } from '../../components/scroll-region';
import DraftListStore from './store';
import DraftSelectedItemsStack from './selected-items-stack';
import DraftThread from './draft-thread';

interface DraftPreviewState {
  headerMessageId: string | null;
  threadId: string | null;
  selectionCount: number;
  isSending: boolean;
}

// Shows the focused draft in the inline composer, below the thread it replies in: the Drafts
// reading pane in split mode, and the body of the pushed Draft sheet in list mode.
export default class DraftPreview extends React.Component<
  Record<string, unknown>,
  DraftPreviewState
> {
  static displayName = 'DraftPreview';
  static containerRequired = false;
  static containerStyles = {
    minWidth: 480,
    maxWidth: 999999,
  };

  _composer: any = null;
  _scrollRegion: ScrollRegion | null = null;
  _composerWrap: HTMLDivElement | null = null;
  _unsubscribers: (() => void)[] = [];
  _selectionSubscription: { dispose: () => void } | null = null;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = this._getStateFromStores(0);
  }

  componentDidMount() {
    this._unsubscribers = [
      DraftListStore.listen(this._onChange),
      DraftStore.listen(this._onChange),
    ];
    this._selectionSubscription = DraftListStore.selectionObservable().subscribe((items) =>
      this.setState({ selectionCount: items ? items.length : 0 })
    );
    DraftListStore.setPreviewFocusHandler(this._focusComposer);
  }

  componentWillUnmount() {
    this._unsubscribers.forEach((unsub) => unsub());
    if (this._selectionSubscription) {
      this._selectionSubscription.dispose();
    }
    // A sheet transition can mount the next preview before this one unmounts.
    DraftListStore.clearPreviewFocusHandler(this._focusComposer);
  }

  _onChange = () => {
    this.setState(this._getStateFromStores(this.state.selectionCount));
  };

  _getStateFromStores(selectionCount: number): DraftPreviewState {
    const headerMessageId = DraftListStore.focusedHeaderMessageId();
    return {
      headerMessageId,
      threadId: DraftListStore.focusedThreadId(),
      selectionCount,
      isSending: headerMessageId ? DraftStore.isSendingDraft(headerMessageId) : false,
    };
  }

  _focusComposer = () => {
    if (this._composer && this._composer.focus) {
      this._composer.focus();
    }
  };

  _scrollToComposer = () => {
    if (this._scrollRegion && this._composerWrap) {
      this._scrollRegion.scrollTo(this._composerWrap, {
        position: ScrollRegion.ScrollPosition.Top,
        settle: true,
      });
    }
  };

  render() {
    const { headerMessageId, threadId, selectionCount, isSending } = this.state;

    if (selectionCount > 1) {
      return <DraftSelectedItemsStack selectionCount={selectionCount} />;
    }
    if (!headerMessageId) {
      return <div className="draft-preview draft-preview-empty" />;
    }
    if (isSending) {
      return (
        <div className="draft-preview draft-preview-empty">
          <div className="draft-preview-status">{localized('Sending message')}</div>
        </div>
      );
    }

    const Composer = ComponentRegistry.findComponentsMatching({ role: 'Composer' })[0];
    if (!Composer) {
      return <div className="draft-preview draft-preview-empty" />;
    }
    return (
      <ScrollRegion
        className="draft-preview"
        ref={(el: ScrollRegion | null) => {
          this._scrollRegion = el;
        }}
      >
        <div className="draft-preview-content">
          {threadId && (
            <DraftThread
              key={`${headerMessageId}-${threadId}`}
              threadId={threadId}
              onShown={this._scrollToComposer}
            />
          )}
          <div
            ref={(el) => {
              this._composerWrap = el;
            }}
          >
            <Composer
              key={headerMessageId}
              ref={(cm: unknown) => {
                this._composer = cm;
              }}
              headerMessageId={headerMessageId}
              className="draft-preview-composer"
              mode="inline"
              keepPristineDraft
              skipInitialFocus
            />
          </div>
        </div>
      </ScrollRegion>
    );
  }
}
