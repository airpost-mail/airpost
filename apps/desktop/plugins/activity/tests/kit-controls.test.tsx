// SPDX-License-Identifier: GPL-3.0-only
// The activity views' controls on the UI kit.
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { FocusedPerspectiveStore } from 'mailspring-exports';
import { ActivityFeedToolbar } from '../lib/feed/toolbar';
import ActivityListButton from '../lib/list/button';
import ActivityEventStore from '../lib/event-store';

const filters = {
  search: '',
  kind: 'all' as const,
  collapseRepeats: false,
  groupBy: 'none' as const,
};

describe('Activity kit controls', () => {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  describe('the feed toolbar', () => {
    it('toggles collapsing repeats with a kit checkbox inside its label', () => {
      const onChange = jasmine.createSpy('onChange');
      const { container } = render(
        <ActivityFeedToolbar
          filters={filters}
          eventCount={3}
          onChange={onChange}
          onExport={() => {}}
        />
      );
      const label = container.querySelector('.toolbar-checkbox') as HTMLElement;
      const box = label.querySelector('[data-slot="checkbox"]');
      expect(box.getAttribute('aria-checked')).toBe('false');
      // A click on the label text reaches the checkbox, as with the native one.
      label.click();
      expect(onChange).toHaveBeenCalledWith({ collapseRepeats: true });
    });

    it('searches from a kit text field', () => {
      const onChange = jasmine.createSpy('onChange');
      const { container } = render(
        <ActivityFeedToolbar
          filters={filters}
          eventCount={3}
          onChange={onChange}
          onExport={() => {}}
        />
      );
      const field = container.querySelector('input[data-slot="input"]') as HTMLInputElement;
      expect(field.type).toBe('search');
      fireEvent.change(field, { target: { value: 'acme' } });
      expect(onChange).toHaveBeenCalledWith({ search: 'acme' });
    });

    it('exports from a kit button that is disabled with no events', () => {
      const onExport = jasmine.createSpy('onExport');
      const { container, rerender } = render(
        <ActivityFeedToolbar
          filters={filters}
          eventCount={0}
          onChange={() => {}}
          onExport={onExport}
        />
      );
      const button = container.querySelector('[data-slot="button"]') as HTMLButtonElement;
      expect(button.textContent).toBe('Export CSV');
      expect(button.disabled).toBe(true);
      rerender(
        <ActivityFeedToolbar
          filters={filters}
          eventCount={2}
          onChange={() => {}}
          onExport={onExport}
        />
      );
      expect(button.disabled).toBe(false);
      button.click();
      expect(onExport).toHaveBeenCalled();
    });
  });

  it('opens the activity list from a kit button in the sidebar toolbar', () => {
    spyOn(ActivityEventStore, 'unreadCount').andReturn(2);
    const { container } = render(<ActivityListButton />);
    const button = container.querySelector('[data-slot="button"]') as HTMLButtonElement;
    expect(button.className).toContain('toolbar-activity');
    expect(button.getAttribute('aria-label')).toBe('View activity (2)');
    expect(button.querySelector('.unread-count.active').textContent).toBe('2');
    // Its tooltip says what it does.
    fireEvent.focus(button);
    expect(document.querySelector('[data-slot="tooltip-content"]').textContent).toContain(
      'View activity'
    );
  });
});

describe('Activity tabs', () => {
  const proxyquire = require('proxyquire').noPreserveCache();
  const stub = (name: string) => {
    const C = () => <div className={`stub-${name}`} />;
    C.displayName = name;
    return { default: C, __esModule: true };
  };
  // The tab panels are stubbed; the specs are about the tab list.
  const Root = proxyquire('../lib/root', {
    './feed/feed': stub('feed'),
    './dashboard/root': stub('reports'),
    './engagement/board': stub('engagement'),
  }).default;

  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
    spyOn(FocusedPerspectiveStore, 'current').andReturn({ accountIds: [TEST_ACCOUNT_ID] } as any);
    AppEnv.savedState.activityTab = undefined;
  });

  afterEach(() => cleanup());

  it('switches panels from kit tabs and keeps every panel mounted', () => {
    const { container } = render(<Root />);
    const tabs = Array.from(container.querySelectorAll('[role="tab"]')) as HTMLElement[];
    expect(tabs.map((t) => t.textContent)).toEqual(['Feed', 'Engagement', 'Reports']);
    expect(tabs.map((t) => t.getAttribute('data-state'))).toEqual([
      'active',
      'inactive',
      'inactive',
    ]);
    const panels = () =>
      Array.from(container.querySelectorAll('.activity-tab-panel')).map((p) =>
        p.getAttribute('data-state')
      );
    expect(panels()).toEqual(['active', 'inactive', 'inactive']);
    expect(container.querySelector('.stub-reports')).not.toBe(null);

    fireEvent.mouseDown(tabs[2], { button: 0 });
    expect(tabs[2].getAttribute('aria-selected')).toBe('true');
    expect(panels()).toEqual(['inactive', 'inactive', 'active']);
    expect(AppEnv.savedState.activityTab).toBe('reports');
  });

  it('keeps the arrow keys that move between tabs from reaching the keymap', () => {
    const { container } = render(<Root />);
    const tab = container.querySelector('[role="tab"]');
    const event = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true });
    tab.dispatchEvent(event);
    expect((event as any).propagationStopped).toBe(true);
  });
});
