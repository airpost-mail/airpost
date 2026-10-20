import React from 'react';
import { Utils, localized } from 'mailspring-exports';
import { IconButton } from '../../../components/kit/icon-button';
import { SegmentedControl } from '../../../components/kit/segmented-control';
import { CalendarView } from '../constants';

const VIEW_LABELS: { [V in CalendarView]: () => string } = {
  [CalendarView.DAY]: () => localized('Day'),
  [CalendarView.WEEK]: () => localized('Week'),
  [CalendarView.MONTH]: () => localized('Month'),
  [CalendarView.AGENDA]: () => localized('Agenda'),
};

const VIEWS = [CalendarView.DAY, CalendarView.WEEK, CalendarView.MONTH, CalendarView.AGENDA];

export class HeaderControls extends React.Component<{
  title: string;
  nextAction: () => void;
  prevAction: () => void;
  onChangeView: (view: CalendarView) => void;
  /** The view on screen, shown as the chosen segment of the view switcher. */
  disabledViewButton: string;
}> {
  static displayName = 'HeaderControls';

  shouldComponentUpdate(nextProps, nextState) {
    return !Utils.isEqualReact(nextProps, this.props) || !Utils.isEqualReact(nextState, this.state);
  }

  _renderNextAction() {
    if (!this.props.nextAction) {
      return false;
    }
    return (
      <IconButton
        className="next"
        icon="ic-calendar-right-arrow.png"
        label={localized('Next')}
        onClick={this.props.nextAction}
      />
    );
  }

  _renderPrevAction() {
    if (!this.props.prevAction) {
      return false;
    }
    return (
      <IconButton
        className="prev"
        icon="ic-calendar-left-arrow.png"
        label={localized('Previous')}
        onClick={this.props.prevAction}
      />
    );
  }

  _changeView = (newView: CalendarView) => {
    if (newView === this.props.disabledViewButton) return;
    this.props.onChangeView(newView);
  };

  // The arrows move the switcher's choice; they don't also move the selected event
  // (calendar:move-event-left/right).
  _onViewControlsKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') e.stopPropagation();
  };

  render() {
    return (
      <div className="header-controls" onClick={(e) => e.stopPropagation()}>
        <div className="side-controls">{this.props.children}</div>
        <div className="center-controls">
          {this._renderPrevAction()}
          <span className="title">{this.props.title}</span>
          {this._renderNextAction()}
        </div>
        <div className="side-controls">
          <div className="view-controls" onKeyDown={this._onViewControlsKeyDown}>
            <SegmentedControl<CalendarView>
              label={localized('Calendar view')}
              value={this.props.disabledViewButton as CalendarView}
              options={VIEWS.map((view) => [view, VIEW_LABELS[view]()])}
              onChange={this._changeView}
            />
          </div>
        </div>
      </div>
    );
  }
}
