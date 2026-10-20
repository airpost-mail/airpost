// SPDX-License-Identifier: GPL-3.0-only
// The registry slots the calendar keeps: plugins can fill each one, so each renders a component
// registered into it.
import React from 'react';
import moment from 'moment';
import Rx from 'rx-lite';
import { render, cleanup } from '@testing-library/react';
import { ComponentRegistry, WorkspaceStore } from 'mailspring-exports';
import Sheet from '../../src/components/sheets/sheet';
import Toolbar from '../../src/components/sheets/toolbar';
import { DayView } from '../../src/features/main-calendar/views/day';
import { WeekView } from '../../src/features/main-calendar/views/week';
import { MonthView } from '../../src/features/main-calendar/views/month';
import { AgendaView } from '../../src/features/main-calendar/views/agenda';
import { CalendarEvent } from '../../src/features/main-calendar/event/event';
import { EventAttendeesInput } from '../../src/features/main-calendar/event/attendees-input';
import { TimedOccurrence } from '../../src/features/main-calendar/data-source';
import * as CalendarFeature from '../../src/features/main-calendar';
import { MainCalendar } from '../../src/features/main-calendar/main-calendar';
import { CalendarSidebar } from '../../src/features/main-calendar/sidebar';
import { QuickEventButton } from '../../src/features/main-calendar/quick-event-button';
import { EventSearchBar } from '../../src/features/main-calendar/search-bar';

const START = Date.UTC(2026, 5, 23, 14, 0, 0) / 1000;

const occurrence = {
  id: 'e1-e0',
  accountId: 'a',
  calendarId: 'c',
  title: 'Planning',
  location: '',
  description: '',
  isAllDay: false,
  isCancelled: false,
  isPending: false,
  isException: false,
  isRecurring: false,
  organizer: null,
  attendees: [],
  start: START,
  end: START + 3600,
} as TimedOccurrence;

const viewProps: any = {
  dataSource: { buildObservable: () => Rx.Observable.just({ events: [] }) },
  disabledCalendars: [],
  focusedMoment: moment(),
  focusedEvent: null,
  selectedEvents: [],
  onChangeView: () => {},
  onChangeFocusedMoment: () => {},
  onCalendarMouseUp: () => {},
  onCalendarMouseDown: () => {},
  onCalendarMouseMove: () => {},
  onCalendarClick: () => {},
  onCalendarDoubleClick: () => {},
  onCalendarContextMenu: () => {},
  onEventClick: () => {},
  onEventDoubleClick: () => {},
  onEventContextMenu: () => {},
  onEventFocused: () => {},
  onEventDragStart: () => {},
  dragState: null,
  newEventPreview: null,
  readOnlyCalendarIds: new Set(),
  isCalendarReadOnly: () => false,
  paintVersion: '1-0',
};

function fake(name: string, seen?: (props: any) => void) {
  const Fake = (props: any) => {
    seen?.(props);
    return <span className={`fake-${name}`} />;
  };
  Fake.displayName = `Fake${name}`;
  return Fake;
}

function withRegistered(
  component: React.ComponentType<any>,
  descriptor: Record<string, unknown>,
  fn: () => void
) {
  ComponentRegistry.register(component, descriptor);
  try {
    fn();
  } finally {
    ComponentRegistry.unregister(component);
  }
}

describe('Calendar registry slots', function () {
  afterEach(cleanup);

  const banners: [string, React.ComponentType<any>, string][] = [
    ['DayView', DayView, 'Calendar:Day:Banner'],
    ['WeekView', WeekView, 'Calendar:Week:Banner'],
    ['MonthView', MonthView, 'Calendar:Week:Banner'],
    ['AgendaView', AgendaView, 'Calendar:Week:Banner'],
  ];
  for (const [name, View, role] of banners) {
    it(`${name} renders the ${role} components in its top banner`, function () {
      const Banner = fake('banner');
      withRegistered(Banner, { role }, () => {
        const { container } = render(<View {...viewProps} />);
        expect(container.querySelector('.top-banner .fake-banner')).not.toBe(null);
      });
    });
  }

  it('renders the Calendar:Event components inside each event, with the event', function () {
    const seen = jasmine.createSpy('props');
    withRegistered(fake('event-decoration', seen), { role: 'Calendar:Event' }, () => {
      const { container } = render(
        <CalendarEvent
          event={occurrence}
          order={1}
          selected={false}
          focused={false}
          scopeStart={START - 3600}
          scopeEnd={START + 86400}
          direction="vertical"
          fixedSize={-1}
          concurrentEvents={1}
          onClick={() => {}}
          onDoubleClick={() => {}}
          onFocused={() => {}}
        />
      );
      expect(container.querySelector('.event-injected-components .fake-event-decoration')).not.toBe(
        null
      );
      expect(seen.mostRecentCall.args[0].event).toBe(occurrence);
    });
  });

  it('renders the Composer:RecipientChip components in each attendee token, with the contact', function () {
    const seen = jasmine.createSpy('props');
    const attendee = { email: 'ben@example.com', name: 'Ben', partstat: 'NEEDS-ACTION' };
    withRegistered(fake('chip', seen), { role: 'Composer:RecipientChip' }, () => {
      const { container } = render(
        <EventAttendeesInput attendees={[attendee]} change={() => {}} className="attendees" />
      );
      expect(container.querySelector('.token .fake-chip')).not.toBe(null);
      expect(seen.mostRecentCall.args[0].contact).toBe(attendee);
    });
  });

  describe('the Calendar sheet', function () {
    beforeEach(function () {
      if (!WorkspaceStore.Sheet.Calendar) {
        WorkspaceStore.defineSheet('Calendar', { root: true, silent: true } as any, {
          list: ['CalendarSidebar', 'CalendarContent'],
        });
      }
      spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
    });

    it('puts the built-in screens into its column and toolbar locations', function () {
      const registered: [any, any][] = [];
      spyOn(ComponentRegistry, 'register').andCallFake((component, descriptor) => {
        registered.push([component, descriptor]);
        return ComponentRegistry;
      });
      spyOn(ComponentRegistry, 'unregister');
      const defineSheet = spyOn(WorkspaceStore, 'defineSheet');
      CalendarFeature.activate();
      CalendarFeature.deactivate();
      const { Location } = WorkspaceStore;
      expect(defineSheet).toHaveBeenCalledWith(
        'Calendar',
        { root: true },
        { list: ['CalendarSidebar', 'CalendarContent'] }
      );
      const at = (component) => registered.find(([c]) => c === component)?.[1];
      expect(at(CalendarSidebar)).toEqual({ location: Location.CalendarSidebar });
      expect(at(MainCalendar)).toEqual({ location: Location.CalendarContent });
      expect(at(QuickEventButton)).toEqual({ location: Location.CalendarSidebar.Toolbar });
      expect(at(EventSearchBar)).toEqual({ location: Location.CalendarContent.Toolbar });
    });

    it('renders components registered into its columns', function () {
      const { Location } = WorkspaceStore;
      const Side = fake('side');
      const Content = fake('content');
      withRegistered(Side, { location: Location.CalendarSidebar }, () =>
        withRegistered(Content, { location: Location.CalendarContent }, () => {
          const { container } = render(<Sheet data={WorkspaceStore.Sheet.Calendar} depth={0} />);
          expect(container.querySelector('.column-CalendarSidebar .fake-side')).not.toBe(null);
          expect(container.querySelector('.column-CalendarContent .fake-content')).not.toBe(null);
        })
      );
    });

    it('renders components registered into its column toolbars', function () {
      const { Location } = WorkspaceStore;
      const SideItem = fake('side-item');
      const ContentItem = fake('content-item');
      withRegistered(SideItem, { location: Location.CalendarSidebar.Toolbar }, () =>
        withRegistered(ContentItem, { location: Location.CalendarContent.Toolbar }, () => {
          const { container } = render(<Toolbar data={WorkspaceStore.Sheet.Calendar} depth={0} />);
          expect(container.querySelector('.toolbar-CalendarSidebar .fake-side-item')).not.toBe(
            null
          );
          expect(container.querySelector('.toolbar-CalendarContent .fake-content-item')).not.toBe(
            null
          );
        })
      );
    });
  });
});
