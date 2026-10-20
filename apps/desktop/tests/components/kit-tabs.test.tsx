// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { DirectionProvider } from '@radix-ui/react-direction';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../../src/components/kit/tabs';
import { wait } from '../support/kit-helpers';

const Sections = (props: {
  orientation?: 'horizontal' | 'vertical';
  onValueChange?: (v: string) => void;
}) => (
  <Tabs defaultValue="one" orientation={props.orientation} onValueChange={props.onValueChange}>
    <TabsList aria-label="Sections" className="custom">
      <TabsTrigger value="one">One</TabsTrigger>
      <TabsTrigger value="two">Two</TabsTrigger>
      <TabsTrigger value="three" disabled>
        Three
      </TabsTrigger>
      <TabsTrigger value="four">Four</TabsTrigger>
    </TabsList>
    <TabsContent value="one">First</TabsContent>
    <TabsContent value="two">Second</TabsContent>
    <TabsContent value="three">Third</TabsContent>
    <TabsContent value="four">Fourth</TabsContent>
  </Tabs>
);

describe('kit Tabs', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('renders the kit markup and state attributes', () => {
    const { container } = render(<Sections />);
    const root = container.querySelector('[data-slot="tabs"]');
    expect(root.getAttribute('dir')).toBe('ltr');
    expect(root.getAttribute('data-orientation')).toBe('horizontal');
    const list = container.querySelector('[data-slot="tabs-list"]');
    expect(list.getAttribute('role')).toBe('tablist');
    expect(list.getAttribute('aria-orientation')).toBe('horizontal');
    expect(list.className).toBe('custom');
    const triggers = Array.from(container.querySelectorAll('[data-slot="tabs-trigger"]'));
    expect(triggers.map((t) => t.getAttribute('role'))).toEqual(['tab', 'tab', 'tab', 'tab']);
    expect(triggers.map((t) => t.getAttribute('data-state'))).toEqual([
      'active',
      'inactive',
      'inactive',
      'inactive',
    ]);
    expect(triggers[0].getAttribute('aria-selected')).toBe('true');
    expect(triggers[2].hasAttribute('data-disabled')).toBe(true);
    const panels = Array.from(container.querySelectorAll('[data-slot="tabs-content"]'));
    expect(panels.map((p) => p.getAttribute('role'))).toEqual([
      'tabpanel',
      'tabpanel',
      'tabpanel',
      'tabpanel',
    ]);
    expect(panels.map((p) => p.getAttribute('data-state'))).toEqual([
      'active',
      'inactive',
      'inactive',
      'inactive',
    ]);
    expect(triggers[0].getAttribute('aria-controls')).toBe(panels[0].id);
    // Inactive panels stay in the DOM, hidden and empty unless forceMount is set.
    expect(panels[0].textContent).toBe('First');
    expect(panels[1].hasAttribute('hidden')).toBe(true);
    expect(panels[1].textContent).toBe('');
  });

  it('switches on click', () => {
    const onValueChange = jasmine.createSpy('onValueChange');
    const { container } = render(<Sections onValueChange={onValueChange} />);
    const triggers = container.querySelectorAll('[data-slot="tabs-trigger"]');
    fireEvent.mouseDown(triggers[1], { button: 0 });
    expect(onValueChange).toHaveBeenCalledWith('two');
    expect(triggers[1].getAttribute('data-state')).toBe('active');
    expect(
      container.querySelector('[data-slot="tabs-content"][data-state="active"]').textContent
    ).toBe('Second');
  });

  // Focuses tab `from`, presses `key` and returns the index of the tab that ends up focused
  // and active.
  const pressArrow = async (ui: React.ReactElement, from: number, key: string) => {
    const { container, unmount } = render(ui);
    const triggers = Array.from(
      container.querySelectorAll('[data-slot="tabs-trigger"]')
    ) as HTMLElement[];
    triggers[from].focus();
    fireEvent.keyDown(triggers[from], { key });
    await wait(20);
    const index = triggers.indexOf(document.activeElement as HTMLElement);
    const active = triggers.findIndex((t) => t.getAttribute('data-state') === 'active');
    unmount();
    return index === active ? index : -1;
  };

  it('moves focus and activates with the arrow keys, looping and skipping disabled tabs', async () => {
    expect(await pressArrow(<Sections />, 0, 'ArrowRight')).toBe(1);
    expect(await pressArrow(<Sections />, 1, 'ArrowRight')).toBe(3);
    expect(await pressArrow(<Sections />, 0, 'ArrowLeft')).toBe(3);
    expect(await pressArrow(<Sections orientation="vertical" />, 0, 'ArrowDown')).toBe(1);
  });

  it('mirrors the arrow keys in RTL', async () => {
    const rtl = (
      <DirectionProvider dir="rtl">
        <Sections />
      </DirectionProvider>
    );
    expect(await pressArrow(rtl, 0, 'ArrowLeft')).toBe(1);
    expect(await pressArrow(rtl, 0, 'ArrowRight')).toBe(3);
    const { container } = render(rtl);
    expect(container.querySelector('[data-slot="tabs"]').getAttribute('dir')).toBe('rtl');
  });
});
