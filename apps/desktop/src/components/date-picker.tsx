import moment, { Moment } from 'moment';
import classnames from 'classnames';
import React from 'react';
import DateUtils from '../utils/date';
import { localized } from '../intl/localize';
import { MiniMonthView } from './mini-month-view';
import { TabGroupContext } from './tab-group-region';

type DatePickerProps = {
  value?: number;
  // Required because defaultProps supplies it; optional for JSX callers.
  onChange: (ms: number) => void;
};
type DatePickerState = {
  focused: boolean;
};

export class DatePicker extends React.Component<DatePickerProps, DatePickerState> {
  static displayName = 'DatePicker';

  static contextType = TabGroupContext;
  context!: React.ContextType<typeof TabGroupContext>;

  static defaultProps = {
    onChange: () => {},
  };

  constructor(props: DatePickerProps) {
    super(props);
    this.state = { focused: false };
  }

  value() {
    return this.props.value ? moment(this.props.value) : null;
  }

  // Apply the time to the destination day rather than moving the value's own date: a
  // same-day-next-year intermediate can land in that year's spring-forward gap and gain an hour.
  _changeDay(day: Moment) {
    const val = this.value();
    if (!val) return;
    const next = day.set({
      hour: val.hour(),
      minute: val.minute(),
      second: val.second(),
      millisecond: val.millisecond(),
    });
    this.props.onChange(next.valueOf());
  }

  _moveDay(numDays: number) {
    const val = this.value();
    if (!val) return;
    this._changeDay(val.add(numDays, 'days'));
  }

  _onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      this._moveDay(-1);
    } else if (event.key === 'ArrowRight') {
      this._moveDay(1);
    } else if (event.key === 'ArrowUp') {
      this._moveDay(-7);
    } else if (event.key === 'ArrowDown') {
      this._moveDay(7);
    } else if (event.key === 'Enter') {
      this.context?.shiftFocus(1);
    }
  };

  _onFocus = () => {
    this.setState({ focused: true });
  };

  _onBlur = () => {
    this.setState({ focused: false });
  };

  _onSelectDay = (newTimestamp: Moment) => {
    this._changeDay(moment(newTimestamp));
    this.context?.shiftFocus(1);
  };

  _renderMiniMonthView() {
    const val = this.value();
    if (this.state.focused && val) {
      return (
        <div className="mini-month-view-wrap">
          <MiniMonthView onChange={this._onSelectDay} value={val} />
        </div>
      );
    }
    return false;
  }

  render() {
    const className = classnames({
      'day-text': true,
      focused: this.state.focused,
    });

    const val = this.value();
    let dayTxt = localized('Click to set date');
    if (val) {
      dayTxt = val.format(DateUtils.DATE_FORMAT_llll_NO_TIME);
    }

    return (
      <div
        tabIndex={0}
        className="date-picker"
        onKeyDown={this._onKeyDown}
        onFocus={this._onFocus}
        onBlur={this._onBlur}
      >
        <div className={className}>{dayTxt}</div>
        {this._renderMiniMonthView()}
      </div>
    );
  }
}
