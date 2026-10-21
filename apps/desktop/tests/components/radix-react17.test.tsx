import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Slot } from '@radix-ui/react-slot';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import * as RadioGroupPrimitive from '@radix-ui/react-radio-group';
import * as SelectPrimitive from '@radix-ui/react-select';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import * as TooltipPrimitive from '@radix-ui/react-tooltip';
import * as TabsPrimitive from '@radix-ui/react-tabs';
import { DirectionProvider } from '@radix-ui/react-direction';
import { VisuallyHidden } from '@radix-ui/react-visually-hidden';

// The Radix primitives the UI kit builds on (docs/desktop/ui-kit.md), run on the app's React 17
// in the renderer, and the Escape contract with the keymap's core:pop-sheet.

const wait = (ms: number) => new Promise<void>((resolve) => window.originalSetTimeout(resolve, ms));

describe('Radix primitives on React 17', function () {
  let portal: HTMLElement;

  beforeEach(() => {
    portal = document.createElement('div');
    portal.setAttribute('data-portal-root', '');
    document.body.appendChild(portal);
  });

  afterEach(() => {
    cleanup();
    portal.remove();
  });

  it('runs on React 17', () => {
    expect(React.version).toBe('17.0.2');
    expect((React as any).useId).toBeUndefined();
  });

  it('Slot merges props and refs into its child', () => {
    // The click would reach the window's link handler, whose "Failed to Open Link" dialog for
    // http://#x blocks the run.
    spyOn(AppEnv.windowEventHandler, 'openLink');
    const ref = React.createRef<HTMLAnchorElement>();
    const onClick = jasmine.createSpy('onClick');
    const { container } = render(
      <Slot data-slot="button" className="kit" onClick={onClick} ref={ref as any}>
        <a href="#x" className="child">
          Link
        </a>
      </Slot>
    );
    const a = container.querySelector('a');
    expect(a.getAttribute('data-slot')).toBe('button');
    expect(a.className).toBe('kit child');
    expect(ref.current).toBe(a);
    fireEvent.click(a);
    expect(onClick).toHaveBeenCalled();
  });

  it('Switch toggles and reports data-state', () => {
    const onCheckedChange = jasmine.createSpy('onCheckedChange');
    const { container } = render(
      <SwitchPrimitive.Root aria-label="Wrap" onCheckedChange={onCheckedChange}>
        <SwitchPrimitive.Thumb />
      </SwitchPrimitive.Root>
    );
    const root = container.querySelector('button');
    expect(root.getAttribute('role')).toBe('switch');
    expect(root.getAttribute('data-state')).toBe('unchecked');
    fireEvent.click(root);
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(root.getAttribute('data-state')).toBe('checked');
    expect(root.getAttribute('aria-checked')).toBe('true');
  });

  it('Checkbox supports the indeterminate state', () => {
    const { container } = render(
      <CheckboxPrimitive.Root aria-label="All" checked="indeterminate">
        <CheckboxPrimitive.Indicator className="indicator" />
      </CheckboxPrimitive.Root>
    );
    const root = container.querySelector('button');
    expect(root.getAttribute('role')).toBe('checkbox');
    expect(root.getAttribute('aria-checked')).toBe('mixed');
    expect(root.getAttribute('data-state')).toBe('indeterminate');
    expect(container.querySelector('.indicator')).not.toBe(null);
  });

  it('RadioGroup moves the choice with the arrow keys, mirrored in RTL', async () => {
    // Roving focus calls setTimeout() with no delay, which the spec clock never fires.
    jasmine.unspy(window, 'setTimeout');
    const Group = (props: { dir: 'ltr' | 'rtl' }) => {
      const [value, setValue] = React.useState('a');
      return (
        <DirectionProvider dir={props.dir}>
          <RadioGroupPrimitive.Root
            value={value}
            onValueChange={setValue}
            orientation="horizontal"
            loop
            aria-label="Density"
          >
            {['a', 'b', 'c'].map((v) => (
              <RadioGroupPrimitive.Item key={v} value={v} id={`radio-${props.dir}-${v}`}>
                {v}
              </RadioGroupPrimitive.Item>
            ))}
          </RadioGroupPrimitive.Root>
        </DirectionProvider>
      );
    };
    for (const [dir, key] of [
      ['ltr', 'ArrowRight'],
      ['rtl', 'ArrowLeft'],
    ] as const) {
      const { container, unmount } = render(<Group dir={dir} />);
      const a = container.querySelector(`#radio-${dir}-a`) as HTMLElement;
      const b = container.querySelector(`#radio-${dir}-b`) as HTMLElement;
      const group = container.querySelector('[role="radiogroup"]') as HTMLElement;
      expect(group.getAttribute('dir')).toBe(dir);
      // Unlike SegmentedControl, the group itself is the tab stop until an item takes focus.
      expect(group.tabIndex).toBe(0);
      expect([a.tabIndex, b.tabIndex]).toEqual([-1, -1]);
      a.focus();
      expect(a.tabIndex).toBe(0);
      fireEvent.keyDown(a, { key });
      // Roving focus moves focus in a setTimeout; the radio checks itself on that focus.
      await wait(10);
      expect(document.activeElement).toBe(b);
      expect(b.getAttribute('aria-checked')).toBe('true');
      fireEvent.keyUp(b, { key });
      unmount();
    }
  });

  it('Select opens into the given portal container and picks a value', () => {
    const onValueChange = jasmine.createSpy('onValueChange');
    const { container } = render(
      <SelectPrimitive.Root defaultValue="a" onValueChange={onValueChange}>
        <SelectPrimitive.Trigger aria-label="Layout">
          <SelectPrimitive.Value />
        </SelectPrimitive.Trigger>
        <SelectPrimitive.Portal container={portal}>
          <SelectPrimitive.Content position="popper">
            <SelectPrimitive.Viewport>
              <SelectPrimitive.Item value="a">
                <SelectPrimitive.ItemText>Alpha</SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
              <SelectPrimitive.Item value="b">
                <SelectPrimitive.ItemText>Beta</SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            </SelectPrimitive.Viewport>
          </SelectPrimitive.Content>
        </SelectPrimitive.Portal>
      </SelectPrimitive.Root>
    );
    const trigger = container.querySelector('button');
    expect(trigger.textContent).toBe('Alpha');
    expect(trigger.getAttribute('data-state')).toBe('closed');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(trigger.getAttribute('data-state')).toBe('open');
    const options = portal.querySelectorAll('[role="option"]');
    expect(options.length).toBe(2);
    fireEvent.click(options[1]);
    // Radix selects on pointerup for mouse and on click/keydown otherwise.
    if (!onValueChange.callCount) fireEvent.keyDown(options[1], { key: 'Enter' });
    expect(onValueChange).toHaveBeenCalledWith('b');
  });

  it('Dialog renders title, description and ids without React.useId', () => {
    const errors = spyOn(console, 'error').andCallThrough();
    render(
      <DialogPrimitive.Root defaultOpen>
        <DialogPrimitive.Portal container={portal}>
          <DialogPrimitive.Overlay className="overlay" />
          <DialogPrimitive.Content>
            <DialogPrimitive.Title>Add account</DialogPrimitive.Title>
            <VisuallyHidden>
              <DialogPrimitive.Description>Choose a provider</DialogPrimitive.Description>
            </VisuallyHidden>
            <DialogPrimitive.Close>Close</DialogPrimitive.Close>
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
    );
    const dialog = portal.querySelector('[role="dialog"]');
    expect(dialog.getAttribute('data-state')).toBe('open');
    const titleId = dialog.getAttribute('aria-labelledby');
    expect(titleId).toMatch(/^radix-/);
    expect(document.getElementById(titleId).textContent).toBe('Add account');
    expect(portal.querySelector('.overlay')).not.toBe(null);
    expect(errors).not.toHaveBeenCalled();
    fireEvent.click(portal.querySelector('button'));
    expect(portal.querySelector('[role="dialog"]')).toBe(null);
  });

  it('DropdownMenu opens from its trigger and selects an item', () => {
    const onSelect = jasmine.createSpy('onSelect');
    const { container } = render(
      <DropdownMenuPrimitive.Root>
        <DropdownMenuPrimitive.Trigger>More</DropdownMenuPrimitive.Trigger>
        <DropdownMenuPrimitive.Portal container={portal}>
          <DropdownMenuPrimitive.Content>
            <DropdownMenuPrimitive.Item onSelect={onSelect}>Archive</DropdownMenuPrimitive.Item>
            <DropdownMenuPrimitive.Separator />
            <DropdownMenuPrimitive.CheckboxItem checked>Starred</DropdownMenuPrimitive.CheckboxItem>
          </DropdownMenuPrimitive.Content>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>
    );
    const trigger = container.querySelector('button');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(trigger.getAttribute('data-state')).toBe('open');
    const items = portal.querySelectorAll('[role^="menuitem"]');
    expect(items.length).toBe(2);
    expect(items[1].getAttribute('data-state')).toBe('checked');
    fireEvent.click(items[0]);
    expect(onSelect).toHaveBeenCalled();
    expect(portal.querySelector('[role="menu"]')).toBe(null);
  });

  it('Tooltip, Tabs and Popover render under a Direction provider', () => {
    portal.setAttribute('dir', 'rtl');
    const { container } = render(
      <DirectionProvider dir="rtl">
        <TooltipPrimitive.Provider>
          <TooltipPrimitive.Root open>
            <TooltipPrimitive.Trigger>?</TooltipPrimitive.Trigger>
            <TooltipPrimitive.Portal container={portal}>
              <TooltipPrimitive.Content className="tip">Help</TooltipPrimitive.Content>
            </TooltipPrimitive.Portal>
          </TooltipPrimitive.Root>
        </TooltipPrimitive.Provider>
        <TabsPrimitive.Root defaultValue="one">
          <TabsPrimitive.List aria-label="Sections">
            <TabsPrimitive.Trigger value="one">One</TabsPrimitive.Trigger>
            <TabsPrimitive.Trigger value="two">Two</TabsPrimitive.Trigger>
          </TabsPrimitive.List>
          <TabsPrimitive.Content value="one">First</TabsPrimitive.Content>
          <TabsPrimitive.Content value="two">Second</TabsPrimitive.Content>
        </TabsPrimitive.Root>
        <PopoverPrimitive.Root defaultOpen>
          <PopoverPrimitive.Trigger>Open</PopoverPrimitive.Trigger>
          <PopoverPrimitive.Portal container={portal}>
            <PopoverPrimitive.Content className="pop">Body</PopoverPrimitive.Content>
          </PopoverPrimitive.Portal>
        </PopoverPrimitive.Root>
      </DirectionProvider>
    );
    expect(portal.querySelector('.tip').getAttribute('data-state')).toMatch(/open/);
    expect(portal.querySelector('.pop').getAttribute('data-state')).toBe('open');
    // Popover content carries no dir of its own: CSS direction comes from the portal container.
    const pop = portal.querySelector('.pop') as HTMLElement;
    expect(pop.getAttribute('dir')).toBe(null);
    expect(getComputedStyle(pop).direction).toBe('rtl');
    const tablist = container.querySelector('[role="tablist"]') as HTMLElement;
    expect(tablist.parentElement.getAttribute('dir')).toBe('rtl');
    const tabs = container.querySelectorAll('[role="tab"]');
    expect(tabs[0].getAttribute('data-state')).toBe('active');
    expect(container.querySelector('[role="tablist"]').getAttribute('aria-orientation')).toBe(
      'horizontal'
    );
    fireEvent.mouseDown(tabs[1], { button: 0 });
    expect(tabs[1].getAttribute('data-state')).toBe('active');
  });

  describe('Keys inside layers and the keymap', function () {
    let popped: number;
    let nextItem: number;
    let handlers: { dispose(): void };

    beforeEach(() => {
      popped = 0;
      nextItem = 0;
      handlers = AppEnv.commands.add(document.body, {
        'core:pop-sheet': () => (popped += 1),
        'core:next-item': () => (nextItem += 1),
      });
    });

    afterEach(() => handlers.dispose());

    const pressEscape = async () => {
      const webContents = AppEnv.getCurrentWindow().webContents;
      webContents.sendInputEvent({ type: 'keyDown', keyCode: 'Escape' });
      webContents.sendInputEvent({ type: 'keyUp', keyCode: 'Escape' });
      await wait(100);
    };

    const renderPopover = (onEscapeKeyDown?: (e: KeyboardEvent) => void) =>
      render(
        <PopoverPrimitive.Root defaultOpen>
          <PopoverPrimitive.Trigger>Open</PopoverPrimitive.Trigger>
          <PopoverPrimitive.Portal container={portal}>
            <PopoverPrimitive.Content className="pop" onEscapeKeyDown={onEscapeKeyDown}>
              Body
            </PopoverPrimitive.Content>
          </PopoverPrimitive.Portal>
        </PopoverPrimitive.Root>
      );

    it('reaches the keymap when a layer only calls preventDefault (Radix default)', async () => {
      await wait(250);
      renderPopover();
      await wait(50);
      expect(portal.contains(document.activeElement)).toBe(true);
      await pressEscape();
      expect(portal.querySelector('.pop')).toBe(null);
      expect(popped).toBeGreaterThan(0);
    });

    it('stays with the layer when onEscapeKeyDown stops propagation', async () => {
      await wait(250);
      renderPopover((e) => e.stopPropagation());
      await wait(50);
      await pressEscape();
      expect(portal.querySelector('.pop')).toBe(null);
      expect(popped).toBe(0);

      // With the layer gone, Escape pops the sheet again.
      await pressEscape();
      expect(popped).toBeGreaterThan(0);
    });

    it('lets plain keys pressed in an open menu reach the keymap (the kit has to guard them)', async () => {
      await wait(250);
      render(
        <DropdownMenuPrimitive.Root defaultOpen>
          <DropdownMenuPrimitive.Trigger>More</DropdownMenuPrimitive.Trigger>
          <DropdownMenuPrimitive.Portal container={portal}>
            <DropdownMenuPrimitive.Content>
              <DropdownMenuPrimitive.Item>One</DropdownMenuPrimitive.Item>
              <DropdownMenuPrimitive.Item>Two</DropdownMenuPrimitive.Item>
            </DropdownMenuPrimitive.Content>
          </DropdownMenuPrimitive.Portal>
        </DropdownMenuPrimitive.Root>
      );
      await wait(50);
      expect(portal.contains(document.activeElement)).toBe(true);
      const webContents = AppEnv.getCurrentWindow().webContents;
      webContents.sendInputEvent({ type: 'keyDown', keyCode: 'Down' });
      webContents.sendInputEvent({ type: 'keyUp', keyCode: 'Down' });
      await wait(100);
      expect(document.activeElement.textContent).toBe('One');
      expect(nextItem).toBeGreaterThan(0);
    });
  });
});
