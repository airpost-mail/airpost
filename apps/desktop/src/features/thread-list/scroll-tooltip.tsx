import React from 'react';
import { localized } from '../../intl/localize';
import DateUtils from '../../utils/date';
import { Thread } from '../../data/models/thread';
import { ScrollRegionTooltipComponentProps } from '../../components/scroll-region';
import ThreadListStore from './store';

function indexFor({ viewportCenter, totalHeight }: ScrollRegionTooltipComponentProps) {
  return Math.floor((ThreadListStore.dataSource().count() / totalHeight) * viewportCenter);
}

const ThreadListScrollTooltip: React.FC<ScrollRegionTooltipComponentProps> = (props) => {
  const idx = indexFor(props);
  const item = ThreadListStore.dataSource().get(idx) as Thread | undefined;
  const content = item
    ? DateUtils.shortTimeString(item.lastMessageReceivedTimestamp)
    : localized('Loading...');
  return <div className="scroll-tooltip">{content}</div>;
};

ThreadListScrollTooltip.displayName = 'ThreadListScrollTooltip';

export default React.memo(
  ThreadListScrollTooltip,
  (prev, next) => indexFor(prev) === indexFor(next)
);
