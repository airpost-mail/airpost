import React from 'react';
import { localized } from 'mailspring-exports';
import {
  DropdownMenu,
  SegmentedControl,
  Button,
  Checkbox,
  TextField,
} from 'mailspring-component-kit';
import { eventCountLabel } from '../events';

export type EventKind = 'all' | 'open' | 'click';
export type GroupBy = 'none' | 'recipient' | 'message';

export interface ActivityFeedFilters {
  search: string;
  kind: EventKind;
  collapseRepeats: boolean;
  groupBy: GroupBy;
}

interface ActivityFeedToolbarProps {
  filters: ActivityFeedFilters;
  eventCount: number;
  onChange: (patch: Partial<ActivityFeedFilters>) => void;
  onExport: () => void;
}

export function ActivityFeedToolbar({
  filters,
  eventCount,
  onChange,
  onExport,
}: ActivityFeedToolbarProps) {
  const { search, kind, collapseRepeats, groupBy } = filters;
  const groupByOptions: { id: GroupBy; name: string }[] = [
    { id: 'none', name: localized('None') },
    { id: 'recipient', name: localized('Recipient') },
    { id: 'message', name: localized('Message') },
  ];
  return (
    <div className="activity-toolbar">
      <TextField
        type="search"
        className="toolbar-search"
        placeholder={localized('Search by recipient, subject, or link…')}
        value={search}
        onChange={(e) => onChange({ search: e.target.value })}
      />
      <SegmentedControl<EventKind>
        className="segmented-control"
        value={kind}
        onChange={(id) => onChange({ kind: id })}
        options={[
          ['all', localized('All')],
          ['open', localized('Opens')],
          ['click', localized('Clicks')],
        ]}
      />
      <div className="group-by">
        <span className="group-by-label">{localized('Group by')}</span>
        <DropdownMenu
          intitialSelectionItem={groupByOptions.find((o) => o.id === groupBy)}
          defaultSelectedIndex={groupByOptions.findIndex((o) => o.id === groupBy)}
          headerComponents={[]}
          footerComponents={[]}
          items={groupByOptions}
          itemKey={(item) => item.id}
          itemContent={(item) => item.name}
          onSelect={(item) => onChange({ groupBy: item.id })}
        />
      </div>
      <label className="toolbar-checkbox">
        <Checkbox
          checked={collapseRepeats}
          onCheckedChange={(checked) => onChange({ collapseRepeats: checked === true })}
        />
        {localized('Collapse repeated events')}
      </label>
      <div className="spacer" />
      <div className="event-count">{eventCountLabel(eventCount)}</div>
      <Button variant="outline" disabled={eventCount === 0} onClick={onExport}>
        {localized('Export CSV')}
      </Button>
    </div>
  );
}
ActivityFeedToolbar.displayName = 'ActivityFeedToolbar';
