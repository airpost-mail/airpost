// SPDX-License-Identifier: GPL-3.0-only
// The calendar's controls on the UI kit.
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { Actions, Calendar, DatabaseStore } from 'mailspring-exports';
import FixedPopover from '../../src/components/fixed-popover';
import { getPortalContainer } from '../../src/components/kit/portal';
import { HeaderControls } from '../../src/features/main-calendar/views/header-controls';
import { CalendarView } from '../../src/features/main-calendar/constants';
import { RepeatSelector } from '../../src/features/main-calendar/event/repeat-selector';
import { CalendarSourceList } from '../../src/features/main-calendar/source-list';
import { EventSearchBar } from '../../src/features/main-calendar/search-bar';
import { QuickEventButton } from '../../src/features/main-calendar/quick-event-button';
import { wait } from '../support/kit-helpers';

describe('Calendar kit controls', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });
  afterEach(() => cleanup());

  describe('HeaderControls', function () {
    const renderHeader = (onChangeView = jasmine.createSpy('onChangeView')) => {
      const nextAction = jasmine.createSpy('next');
      const prevAction = jasmine.createSpy('prev');
      const utils = render(
        <HeaderControls
          title="June 2026"
          nextAction={nextAction}
          prevAction={prevAction}
          onChangeView={onChangeView}
          disabledViewButton={CalendarView.WEEK}
        />
      );
      return { ...utils, onChangeView, nextAction, prevAction };
    };

    it('draws previous and next as kit icon buttons', function () {
      const { container, nextAction, prevAction } = renderHeader();
      const prev = container.querySelector('[data-slot="icon-button"].prev') as HTMLElement;
      const next = container.querySelector('[data-slot="icon-button"].next') as HTMLElement;
      expect(prev.getAttribute('aria-label')).toBe('Previous');
      expect(next.getAttribute('aria-label')).toBe('Next');
      fireEvent.click(prev);
      fireEvent.click(next);
      expect(prevAction).toHaveBeenCalled();
      expect(nextAction).toHaveBeenCalled();
    });

    it('switches views with a segmented control whose chosen segment is the current view', function () {
      const { container, onChangeView } = renderHeader();
      const group = container.querySelector('[data-slot="segmented-control"]');
      expect(group.getAttribute('role')).toBe('radiogroup');
      const segments = Array.from(
        group.querySelectorAll('[data-slot="segmented-control-item"]')
      ) as HTMLElement[];
      expect(segments.map((s) => s.textContent)).toEqual(['Day', 'Week', 'Month', 'Agenda']);
      expect(segments.map((s) => s.getAttribute('aria-checked'))).toEqual([
        'false',
        'true',
        'false',
        'false',
      ]);
      fireEvent.click(segments[1]);
      expect(onChangeView).not.toHaveBeenCalled();
      fireEvent.click(segments[2]);
      expect(onChangeView).toHaveBeenCalledWith(CalendarView.MONTH);
    });

    it('keeps the arrow keys that move the choice from reaching the keymap', function () {
      const { container, onChangeView } = renderHeader();
      const onDocumentKeyDown = jasmine.createSpy('keydown');
      document.addEventListener('keydown', onDocumentKeyDown);
      try {
        const chosen = container.querySelector('[aria-checked="true"]') as HTMLElement;
        fireEvent.keyDown(chosen, { key: 'ArrowRight' });
        expect(onChangeView).toHaveBeenCalledWith(CalendarView.MONTH);
        expect(onDocumentKeyDown).not.toHaveBeenCalled();
        fireEvent.keyDown(chosen, { key: 'Enter' });
        expect(onDocumentKeyDown).toHaveBeenCalled();
      } finally {
        document.removeEventListener('keydown', onDocumentKeyDown);
      }
    });
  });

  describe('an event property select inside the event popover', function () {
    beforeEach(() => spyOn(Actions, 'closePopover'));

    it('picks a value without closing the popover around it', async function () {
      const onChange = jasmine.createSpy('onChange');
      render(
        <FixedPopover originRect={{ top: 100, left: 100, width: 40, height: 20 }}>
          <RepeatSelector value="none" onChange={onChange} />
        </FixedPopover>
      );
      await wait(50);
      const trigger = getPortalContainer().querySelector(
        '[data-slot="select-trigger"].repeat-select'
      ) as HTMLElement;
      expect(trigger.getAttribute('aria-label')).toBe('Repeat');
      expect(trigger.textContent).toBe('None');
      fireEvent.keyDown(trigger, { key: 'Enter' });
      await wait(50);
      const items = Array.from(
        getPortalContainer().querySelectorAll('[data-slot="select-item"]')
      ) as HTMLElement[];
      expect(items.map((i) => i.textContent)).toEqual([
        'None',
        'Every Day',
        'Every Week',
        'Every Month',
        'Every Year',
      ]);
      fireEvent.keyDown(document.activeElement, { key: 'ArrowDown' });
      await wait(20);
      fireEvent.keyDown(document.activeElement, { key: 'Enter' });
      expect(onChange).toHaveBeenCalledWith('daily');
      await wait(50);
      expect(Actions.closePopover).not.toHaveBeenCalled();
    });
  });

  describe('CalendarSourceList', function () {
    const calendars = [
      new Calendar({ id: 'cal-1', accountId: 'acc', name: 'Work' } as any),
      new Calendar({ id: 'cal-2', accountId: 'acc', name: 'Home' } as any),
    ];
    const accounts = [{ id: 'acc', label: 'me@example.com' } as any];

    it('shows each calendar as a kit checkbox in its own colour, toggled from its row', function () {
      spyOn(AppEnv.config, 'get').andReturn(['cal-2']);
      spyOn(AppEnv.config, 'set');
      const { container } = render(
        <CalendarSourceList
          accounts={accounts}
          calendars={calendars}
          disabledCalendars={['cal-2']}
        />
      );
      const boxes = Array.from(
        container.querySelectorAll('[data-slot="checkbox"].colored-checkbox')
      ) as HTMLElement[];
      expect(boxes.map((b) => b.getAttribute('aria-checked'))).toEqual(['true', 'false']);
      expect(boxes[0].style.getPropertyValue('--primary')).not.toBe('');

      const rows = Array.from(container.querySelectorAll('label.toggle-wrap')) as HTMLElement[];
      expect(rows.map((r) => r.textContent)).toEqual(['Work', 'Home']);
      fireEvent.click(rows[1].querySelector('.calendar-name'));
      expect(AppEnv.config.set).toHaveBeenCalledWith('core.calendar.disabledCalendars', []);
    });
  });

  describe('EventSearchBar', function () {
    beforeEach(() => {
      spyOn(DatabaseStore, 'findAll').andReturn(Promise.resolve([]) as any);
    });

    it('clears the query from a kit icon button without taking focus from the field', async function () {
      const { container } = render(<EventSearchBar />);
      const input = container.querySelector('input[data-slot="input"]') as HTMLInputElement;
      expect(input.classList.contains('event-search-input')).toBe(true);
      expect(container.querySelector('label.search-accessory-label').getAttribute('for')).toBe(
        input.id
      );
      input.focus();
      fireEvent.change(input, { target: { value: 'p' } });
      const clear = container.querySelector('[data-slot="icon-button"].clear') as HTMLElement;
      expect(clear.getAttribute('aria-label')).toBe('Clear search');
      let notPrevented: boolean;
      act(() => {
        notPrevented = fireEvent.mouseDown(clear);
      });
      expect(notPrevented).toBe(false);
      expect(input.value).toBe('');
      expect(document.activeElement).toBe(input);
    });
  });

  it('draws the new-event toolbar button as a kit toolbar icon button', function () {
    const { container } = render(<QuickEventButton />);
    const button = container.querySelector('[data-slot="icon-button"]') as HTMLElement;
    expect(button.getAttribute('data-variant')).toBe('toolbar');
    expect(button.classList.contains('btn-toolbar')).toBe(true);
    expect(button.classList.contains('item-compose')).toBe(true);
    expect(button.getAttribute('aria-label')).toBe('Create new event');
  });
});
