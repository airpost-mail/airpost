import _ from 'underscore';
import React, { Component, CSSProperties } from 'react';
import * as Utils from '../../data/models/utils';
import { Model } from '../../data/models/model';
import ReactDOM from 'react-dom';

import { ScrollRegion, ScrollRegionProps } from '../scroll-region';
import { Spinner } from '../spinner';

export * from './data-source';

import { ListDataSource } from './data-source';
import { ListSelection } from './selection';
import { ListTabularItem, ListTabularItemProps } from './tabular-item';

export class ListTabularColumn {
  name: string;
  flex?: number;
  width?: number;
  resolver: any;

  constructor({
    name,
    resolver,
    flex,
    width,
  }: {
    name: string;
    flex?: number;
    width?: number;
    resolver: any;
  }) {
    this.name = name;
    this.resolver = resolver;
    this.flex = flex;
    this.width = width;
  }
}

export interface ListTabularRow {
  item: Model;
  idx: number;
  itemProps?: ListTabularItemProps['itemProps'];
}

type ListTabularRowsProps = {
  rows: ListTabularRow[];
  columns: any[];
  draggable?: boolean;
  itemHeight: number;
  innerStyles?: CSSProperties;
  role?: string;
  ariaLabel?: string;
  ariaMultiselectable?: boolean;
  tabIndex?: number;
  ariaActiveDescendant?: string;
  domRef?: (el: HTMLElement | null) => void;
  onSelect?: (...args: any[]) => any;
  onClick?: (...args: any[]) => any;
  onDoubleClick?: (...args: any[]) => any;
  onDragStart?: (...args: any[]) => any;
  onDragEnd?: (...args: any[]) => any;
};

export const ListTabularRows: React.FC<ListTabularRowsProps> = React.memo(
  ({
    rows,
    columns,
    itemHeight,
    innerStyles,
    draggable,
    role,
    ariaLabel,
    ariaMultiselectable,
    tabIndex,
    ariaActiveDescendant,
    domRef,
    onClick,
    onSelect,
    onDoubleClick,
    onDragStart,
    onDragEnd,
  }) => (
    <div
      ref={domRef}
      className="list-rows"
      style={innerStyles}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      draggable={draggable}
      role={role}
      aria-label={ariaLabel}
      aria-multiselectable={ariaMultiselectable}
      tabIndex={tabIndex}
      aria-activedescendant={ariaActiveDescendant}
    >
      {rows.map(({ item, idx, itemProps = {} }) => {
        if (!item) return null;
        return (
          <ListTabularItem
            key={item.id || idx}
            item={item}
            itemProps={itemProps}
            metrics={{ top: idx * itemHeight, height: itemHeight }}
            columns={columns}
            onSelect={onSelect}
            onClick={onClick}
            onDoubleClick={onDoubleClick}
          />
        );
      })}
    </div>
  ),
  (prev, next) => Utils.isEqualReact(prev, next)
);
ListTabularRows.displayName = 'ListTabularRows';

export interface ListTabularProps extends ScrollRegionProps {
  footer?: React.ReactNode;
  draggable?: boolean;
  className?: string;
  columns: ListTabularColumn[];
  dataSource?: ListDataSource;
  itemPropsProvider?: (...args: any[]) => any;
  itemHeight?: number;
  EmptyComponent?: React.ComponentType<{ visible: boolean }>;
  role?: string;
  ariaLabel?: string;
  ariaMultiselectable?: boolean;
  tabIndex?: number;
  ariaActiveDescendant?: string;
  onClick?: (...args: any[]) => any;
  onSelect?: (...args: any[]) => any;
  onDoubleClick?: (...args: any[]) => any;
  onDragStart?: (...args: any[]) => any;
  onDragEnd?: (...args: any[]) => any;
  onComponentDidUpdate?: (...args: any[]) => any;
}

interface ListTabularState {
  items: { [idx: number]: Model | null };
  animatingOut: {
    [index: string]: {
      idx?: string;
      end: number;
      item: Model;
    };
  };
  renderedRangeStart: number;
  renderedRangeEnd: number;
  count: number;
  loaded: boolean;
  empty: boolean;
}

export class ListTabular extends Component<ListTabularProps, ListTabularState> {
  static displayName = 'ListTabular';

  static defaultProps = {
    footer: false,
    EmptyComponent: () => null,
    itemPropsProvider: () => ({}),
  };

  static Item = ListTabularItem;
  static Column = ListTabularColumn;
  static Selection = ListSelection;
  static DataSource = ListDataSource;

  _unlisten = () => {};
  updateRangeStateFiring = false;
  _cleanupAnimationTimeout?: number | null;
  _onWindowResize?: any;
  _scrollRegion: ScrollRegion | null = null;
  _listRowsEl: HTMLElement | null = null;

  _setListRowsEl = (el: HTMLElement | null) => {
    this._listRowsEl = el;
  };

  constructor(props: ListTabularProps) {
    super(props);
    if (!props.itemHeight) {
      throw new Error(
        'ListTabular: You must provide an itemHeight - raising to avoid divide by zero errors.'
      );
    }

    this.state = this.buildStateForRange({ start: -1, end: -1 });
  }

  // dataSource and itemHeight are optional in the props type but required in practice.
  _dataSource(): ListDataSource {
    const { dataSource } = this.props;
    if (!dataSource) {
      throw new Error('ListTabular: You must provide a dataSource.');
    }
    return dataSource;
  }

  _itemHeight(): number {
    const { itemHeight } = this.props;
    if (!itemHeight) {
      throw new Error(
        'ListTabular: You must provide an itemHeight - raising to avoid divide by zero errors.'
      );
    }
    return itemHeight;
  }

  componentDidMount() {
    window.addEventListener('resize', this.onWindowResize, true);
    this.setupDataSource(this._dataSource());
  }

  componentDidUpdate(prevProps: ListTabularProps, prevState: ListTabularState) {
    if (this.props.onComponentDidUpdate) {
      this.props.onComponentDidUpdate();
    }
    // If our view has been swapped out for an entirely different one,
    // reset our scroll position to the top.
    if (prevProps.dataSource !== this.props.dataSource) {
      if (this._scrollRegion) this._scrollRegion.scrollTop = 0;
      this.setupDataSource(this._dataSource());
    }

    if (this.updateRangeStateFiring) {
      this.updateRangeStateFiring = false;
    } else if (
      prevState.count !== this.state.count ||
      prevProps.itemHeight !== this.props.itemHeight
    ) {
      this.updateRangeStateIfViewportChanged();
    }

    if (!this._cleanupAnimationTimeout) {
      this._cleanupAnimationTimeout = window.setTimeout(this.onCleanupAnimatingItems, 50);
    }
  }

  componentWillUnmount() {
    window.removeEventListener('resize', this.onWindowResize, true);
    if (this._cleanupAnimationTimeout) {
      window.clearTimeout(this._cleanupAnimationTimeout);
    }
    this._unlisten();
  }

  onWindowResize = () => {
    if (this._onWindowResize == null) {
      this._onWindowResize = _.debounce(this.updateRangeStateIfViewportChanged, 50);
    }
    this._onWindowResize();
  };

  onScroll = () => {
    // If we've shifted enough pixels from our previous scrollTop to require
    // new rows to be rendered, update our state!
    this.updateRangeStateIfViewportChanged();
  };

  onCleanupAnimatingItems = () => {
    this._cleanupAnimationTimeout = null;

    const nextAnimatingOut: ListTabularState['animatingOut'] = {};
    Object.entries(this.state.animatingOut).forEach(([idx, record]) => {
      if (Date.now() < record.end) {
        nextAnimatingOut[idx] = record;
      }
    });

    if (Object.keys(nextAnimatingOut).length < Object.keys(this.state.animatingOut).length) {
      this.setState({ animatingOut: nextAnimatingOut });
    }

    if (Object.keys(nextAnimatingOut).length > 0) {
      this._cleanupAnimationTimeout = window.setTimeout(this.onCleanupAnimatingItems, 50);
    }
  };

  setupDataSource(dataSource: ListDataSource) {
    this._unlisten();
    this._unlisten = dataSource.listen(() => this.setState(this.buildStateForRange()), this);

    const range = this.getRange();
    if (range) {
      dataSource.setRetainedRange(range);
    }
    this.setState(this.buildStateForRange({ ...range, dataSource }));
  }

  getRowsToRender() {
    const itemPropsProvider =
      this.props.itemPropsProvider || ListTabular.defaultProps.itemPropsProvider;
    const { items, animatingOut, renderedRangeStart, renderedRangeEnd } = this.state;
    // The ordering of the rows array is important. We want current rows to
    // slide over rows which are animating out, so we need to render them last.
    const rows: ListTabularRow[] = [];
    Object.entries(animatingOut).forEach(([idx, record]) => {
      const itemProps = itemPropsProvider(record.item, Number(idx));
      rows.push({ item: record.item, idx: Number(idx) / 1, itemProps });
    });

    Utils.range(renderedRangeStart, renderedRangeEnd).forEach((idx) => {
      const item = items[idx];
      if (item) {
        const itemProps = itemPropsProvider(item, idx);
        rows.push({ item, idx, itemProps });
      }
    });

    return rows;
  }

  scrollTo(node: HTMLElement) {
    if (!this._scrollRegion) {
      return;
    }
    this._scrollRegion.scrollTo(node);
  }

  focusListbox() {
    this._listRowsEl?.focus({ preventScroll: true });
  }

  scrollByPage(direction: number) {
    if (!this._scrollRegion) {
      return;
    }
    const height = (ReactDOM.findDOMNode(this._scrollRegion) as HTMLElement).clientHeight;
    this._scrollRegion.scrollTop += height * direction;
  }

  getRange() {
    if (!this._scrollRegion) {
      return;
    }
    const { scrollTop } = this._scrollRegion;
    const itemHeight = this._itemHeight();

    // Determine the exact range of rows we want onscreen
    const rangeSize = Math.ceil(window.innerHeight / itemHeight);
    let rangeStart = Math.floor(scrollTop / itemHeight);
    let rangeEnd = rangeStart + rangeSize;

    // Expand the start/end so that you can advance the keyboard cursor fast and
    // we have items to move to and then scroll to.
    rangeStart = Math.max(0, rangeStart - 2);
    rangeEnd = Math.min(rangeEnd + 2, this.state.count + 1);
    return { start: rangeStart, end: rangeEnd };
  }

  updateRangeStateIfViewportChanged() {
    const range = this.getRange();
    if (!range) {
      return;
    }

    // Final sanity check to prevent needless work
    if (
      range.end !== this.state.renderedRangeEnd ||
      range.start !== this.state.renderedRangeStart
    ) {
      this.updateRangeStateFiring = true;
      this._dataSource().setRetainedRange(range);
      this.setState(this.buildStateForRange(range));
    }
  }

  buildStateForRange(
    args: { start?: number; end?: number; dataSource?: ListDataSource } = {}
  ): ListTabularState {
    const {
      start = this.state.renderedRangeStart,
      end = this.state.renderedRangeEnd,
      dataSource = this._dataSource(),
    } = args;

    const items: ListTabularState['items'] = {};
    let animatingOut: ListTabularState['animatingOut'] = {};

    Utils.range(start, end).forEach((idx) => {
      items[idx] = dataSource.get(idx);
    });

    // If we have a previous state, and the previous range matches the new range,
    // (eg: we're not scrolling), identify removed items. We'll render them in one
    // last time but not allocate height to them. This allows us to animate them
    // being covered by other items, not just disappearing when others start to slide up.
    if (this.state && start === this.state.renderedRangeStart) {
      const nextIds = Object.values(items).map((a) => a && a.id);
      animatingOut = {};

      // Keep items which are still animating out and are still not in the set
      Object.entries(this.state.animatingOut).forEach(([recordIdx, record]) => {
        if (Date.now() < record.end && !nextIds.includes(record.item.id)) {
          animatingOut[recordIdx] = record;
        }
      });

      // Add items which are no longer found in the set
      Object.entries(this.state.items).forEach(([previousIdx, previousItem]) => {
        if (!previousItem || nextIds.includes(previousItem.id)) {
          return;
        }
        animatingOut[previousIdx] = {
          idx: previousIdx,
          item: previousItem,
          end: Date.now() + 125,
        };
      });

      // If we think /all/ the items are animating out, or a lot of them,
      // the user probably switched to an entirely different perspective.
      // Don't bother trying to animate.
      const animatingCount = Object.keys(animatingOut).length;
      if (animatingCount > 8 || animatingCount === Object.keys(this.state.items).length) {
        animatingOut = {};
      }
    }

    return {
      items,
      animatingOut,
      renderedRangeStart: start,
      renderedRangeEnd: end,
      count: dataSource.count(),
      loaded: dataSource.loaded(),
      empty: dataSource.empty(),
    };
  }

  render() {
    const {
      footer,
      columns,
      className,
      draggable,
      scrollTooltipComponent,
      role,
      ariaLabel,
      ariaMultiselectable,
      tabIndex,
      ariaActiveDescendant,
      onClick,
      onSelect,
      onDragEnd,
      onDragStart,
      onDoubleClick,
    } = this.props;
    const { count, loaded, empty } = this.state;
    const rows = this.getRowsToRender();
    const itemHeight = this._itemHeight();
    const EmptyComponent = this.props.EmptyComponent || ListTabular.defaultProps.EmptyComponent;

    return (
      <div className={`list-container list-tabular ${className}`}>
        <ScrollRegion
          ref={(cm: ScrollRegion | null) => {
            this._scrollRegion = cm;
          }}
          onScroll={this.onScroll}
          tabIndex={-1}
          scrollTooltipComponent={scrollTooltipComponent}
        >
          <ListTabularRows
            rows={rows}
            columns={columns}
            draggable={draggable}
            itemHeight={itemHeight}
            role={role}
            ariaLabel={ariaLabel}
            ariaMultiselectable={ariaMultiselectable}
            tabIndex={tabIndex}
            ariaActiveDescendant={ariaActiveDescendant}
            domRef={this._setListRowsEl}
            innerStyles={{
              height: count * itemHeight,
              backgroundSize: `100% ${this.props.itemHeight}px`,
            }}
            onClick={onClick}
            onSelect={onSelect}
            onDragEnd={onDragEnd}
            onDragStart={onDragStart}
            onDoubleClick={onDoubleClick}
          />
          <div className="footer">{footer}</div>
        </ScrollRegion>
        <Spinner visible={!loaded && empty} />
        <EmptyComponent visible={loaded && empty} />
      </div>
    );
  }
}
