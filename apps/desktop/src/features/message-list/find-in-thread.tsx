import React from 'react';
import classnames from 'classnames';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { MessageStore } from '../../data/stores/message';
import SearchableComponentStore from '../../data/stores/searchable-component';
import { KeyCommandsRegion } from '../../components/key-commands-region';
import { IconButton } from '../../components/kit/icon-button';
import { TextField } from '../../components/kit/text-field';

interface FindInThreadProps {
  searchTerm: string;
  globalIndex: number;
  resultsLength: number;
}
export default class FindInThread extends React.Component<
  Record<string, unknown>,
  FindInThreadProps
> {
  static displayName = 'FindInThread';

  _usub: () => void;
  _searchBoxEl: HTMLElement;
  _controlsWrapEl: HTMLElement;
  _globalKeymapHandlers: { [command: string]: () => void };

  constructor(props) {
    super(props);
    this.state = SearchableComponentStore.getCurrentSearchData();
    this._globalKeymapHandlers = {
      'core:find-in-thread': this._onFindInThread,
      'core:find-in-thread-next': this._onNextResult,
      'core:find-in-thread-previous': this._onPrevResult,
    };
  }

  componentDidMount() {
    this._usub = SearchableComponentStore.listen(this._onSearchableChange);
  }

  componentWillUnmount() {
    this._usub();
  }

  _onFindInThread = () => {
    if (this.state.searchTerm === null) {
      Actions.findInThread('');
      if (MessageStore.hasCollapsedItems()) {
        Actions.toggleAllMessagesExpanded();
      }
    }
    this._focusSearch();
  };

  _onSearchableChange = () => {
    this.setState(SearchableComponentStore.getCurrentSearchData());
  };

  _onFindChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    Actions.findInThread(event.target.value);
  };

  _onFindKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      return event.shiftKey ? this._onPrevResult() : this._onNextResult();
    } else if (event.key === 'Escape') {
      this._clearSearch();
      this._searchBoxEl.blur();
    }
    return null;
  };

  _selectionText() {
    if (this.state.globalIndex !== null && this.state.resultsLength > 0) {
      return localized('%1$@ of %2$@', this.state.globalIndex + 1, this.state.resultsLength);
    }
    return '';
  }

  _navEnabled() {
    return this.state.resultsLength > 0;
  }

  _onPrevResult = () => {
    if (this._navEnabled()) {
      Actions.previousSearchResult();
    }
  };

  _onNextResult = () => {
    if (this._navEnabled()) {
      Actions.nextSearchResult();
    }
  };

  _clearSearch = () => {
    Actions.findInThread(null);
  };

  _focusSearch = (event?: React.MouseEvent<HTMLDivElement, MouseEvent>) => {
    if (!event || !(this._controlsWrapEl && this._controlsWrapEl.contains(event.currentTarget))) {
      this._searchBoxEl.focus();
    }
  };

  render() {
    const rootCls = classnames({
      'find-in-thread': true,
      enabled: this.state.searchTerm !== null,
    });
    return (
      <div className={rootCls} onClick={this._focusSearch}>
        <KeyCommandsRegion globalHandlers={this._globalKeymapHandlers}>
          <div
            className="controls-wrap"
            ref={(el) => {
              this._controlsWrapEl = el;
            }}
          >
            <div className="input-wrap">
              <TextField
                ref={(el) => {
                  this._searchBoxEl = el;
                }}
                aria-label={localized('Find in thread')}
                placeholder={localized('Find in thread')}
                onChange={this._onFindChange}
                onKeyDown={this._onFindKeyDown}
                value={this.state.searchTerm || ''}
              />

              <div className="selection-progress">{this._selectionText()}</div>

              <div className="btn-wrap">
                <IconButton
                  className="btn-find-in-thread"
                  size="sm"
                  icon="ic-findinthread-previous.png"
                  label={localized('Previous result')}
                  disabled={!this._navEnabled()}
                  onClick={this._onPrevResult}
                />
                <IconButton
                  className="btn-find-in-thread"
                  size="sm"
                  icon="ic-findinthread-next.png"
                  label={localized('Next result')}
                  disabled={!this._navEnabled()}
                  onClick={this._onNextResult}
                />
              </div>
            </div>

            <IconButton
              className="btn-find-in-thread"
              size="sm"
              icon="ic-findinthread-close.png"
              label={localized('Close')}
              onClick={this._clearSearch}
            />
          </div>
        </KeyCommandsRegion>
      </div>
    );
  }
}
