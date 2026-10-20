// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import MultiselectDropdown from '../../src/components/multiselect-dropdown';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { trackCommands } from '../support/kit-layer-helpers';

type Account = { id: string; email: string };
const ACCOUNTS: Account[] = [
  { id: 'a', email: 'a@example.com' },
  { id: 'b', email: 'b@example.com' },
];

const items = () =>
  Array.from(
    getPortalContainer().querySelectorAll<HTMLElement>('[data-slot="dropdown-menu-checkbox-item"]')
  );

function Picker(props: { onToggle?: (a: Account) => void; attachment?: string }) {
  const [checked, setChecked] = React.useState<string[]>(['a']);
  return (
    <MultiselectDropdown
      className="account-dropdown"
      items={ACCOUNTS}
      itemKey={(a: Account) => a.id}
      itemContent={(a: Account) => a.email}
      itemChecked={(a: Account) => checked.includes(a.id)}
      buttonText={`${checked.length} accounts`}
      attachment={props.attachment}
      onToggleItem={(a: Account) => {
        props.onToggle?.(a);
        setChecked((c) => (c.includes(a.id) ? c.filter((id) => id !== a.id) : [...c, a.id]));
      }}
    />
  );
}

describe('MultiselectDropdown on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  const openMenu = (container: HTMLElement) =>
    fireEvent.keyDown(container.querySelector('.only-item'), { key: 'Enter' });

  it('renders the legacy button classes and opens a menu of checkbox items', () => {
    const { container } = render(<Picker />);
    const root = container.querySelector('.button-dropdown');
    expect(root.className).toBe('button-dropdown btn-multiselect bordered account-dropdown');
    const button = root.querySelector('.only-item');
    expect(button.getAttribute('data-slot')).toBe('dropdown-menu-trigger');
    expect(button.textContent).toBe('1 accounts');
    expect(items().length).toBe(0);

    openMenu(container);
    expect(root.className).toContain('open open-down');
    const content = getPortalContainer().querySelector('[data-slot="dropdown-menu-content"]');
    expect(content.className).toBe('menu secondary-items left');
    expect(content.getAttribute('data-align')).toBe('start');
    expect(items().map((i) => i.textContent)).toEqual(['a@example.com', 'b@example.com']);
    expect(items().map((i) => i.getAttribute('data-state'))).toEqual(['checked', 'unchecked']);
    expect(items().map((i) => i.classList.contains('checked'))).toEqual([true, false]);
  });

  it('aligns to the end for another attachment', () => {
    const { container } = render(<Picker attachment="right" />);
    openMenu(container);
    const content = getPortalContainer().querySelector('[data-slot="dropdown-menu-content"]');
    expect(content.getAttribute('data-align')).toBe('end');
  });

  it('toggles an item on click and stays open', () => {
    const onToggle = jasmine.createSpy('onToggle');
    const { container } = render(<Picker onToggle={onToggle} />);
    openMenu(container);
    fireEvent.click(items()[1]);
    expect(onToggle).toHaveBeenCalledWith(ACCOUNTS[1]);
    expect(items().map((i) => i.getAttribute('data-state'))).toEqual(['checked', 'checked']);
    expect(container.querySelector('.only-item').textContent).toBe('2 accounts');
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it('closes the menu without popping the sheet', async () => {
      const { container } = render(<Picker />);
      openMenu(container);
      await wait(20);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBe(0);
      // The menu stays mounted while its close animation runs.
      await wait(300);
      expect(items().length).toBe(0);
      expect(container.querySelector('.button-dropdown').className).not.toContain('open');
      expect(commands.count('core:pop-sheet')).toBe(0);
    });
  });
});
