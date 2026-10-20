// SPDX-License-Identifier: GPL-3.0-only
// Preferences on the UI kit: the registry slots and tab store it keeps, its kit controls, and
// Import from Mailspring at the page level of Accounts.
import React from 'react';
import { ipcRenderer } from 'electron';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import {
  Account,
  AccountStore,
  Actions,
  ComponentRegistry,
  PreferencesUIStore,
  WorkspaceStore,
} from 'mailspring-exports';
import Sheet from '../../src/components/sheets/sheet';
import { getPortalContainer } from '../../src/components/kit/portal';
import { SettingsSelect } from '../../src/components/settings-select';
import ScenarioEditorRow from '../../src/components/scenario-editor/row';
import { Template } from '../../src/components/scenario-editor/models';
import * as PreferencesFeature from '../../src/features/preferences';
import PreferencesRoot from '../../src/features/preferences/root';
import PreferencesAccounts from '../../src/features/preferences/tabs/accounts/accounts';
import PreferencesAppearance from '../../src/features/preferences/tabs/appearance';
import LanguageRow from '../../src/features/preferences/tabs/language-row';

function fake(name: string) {
  const Fake = () => <span className={`fake-${name}`} />;
  Fake.displayName = `Fake${name}`;
  return Fake;
}

function selectItems() {
  return Array.from(
    getPortalContainer().querySelectorAll<HTMLElement>('[data-slot="select-item"]')
  );
}

/** Opens a kit Select from its trigger and clicks the item with `text`. */
function choose(trigger: Element, text: string) {
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const item = selectItems().find((i) => i.textContent === text);
  expect(item).toBeDefined();
  fireEvent.click(item);
}

describe('Preferences slots', function () {
  afterEach(cleanup);

  it('puts PreferencesRoot into the Preferences sheet location', function () {
    spyOn(PreferencesUIStore, 'registerPreferencesTab');
    const register = spyOn(ComponentRegistry, 'register');
    PreferencesFeature.activate();
    expect(WorkspaceStore.Sheet.Preferences).toBeDefined();
    const call = register.calls.find((c) => c.args[0] === PreferencesRoot);
    expect(call.args[1]).toEqual({ location: WorkspaceStore.Location.Preferences });
  });

  it('renders what is registered at the Preferences location in its sheet', function () {
    if (!WorkspaceStore.Sheet.Preferences) {
      WorkspaceStore.defineSheet('Preferences', { hideToolbar: true } as any, {
        list: ['Preferences'],
        split: ['Preferences'],
        splitVertical: ['Preferences'],
      });
    }
    const Pane = fake('preferences-pane');
    ComponentRegistry.register(Pane, { location: WorkspaceStore.Location.Preferences });
    try {
      const { container } = render(<Sheet data={WorkspaceStore.Sheet.Preferences} depth={0} />);
      expect(container.querySelector('.fake-preferences-pane')).not.toBe(null);
    } finally {
      ComponentRegistry.unregister(Pane);
    }
  });

  it("renders a plugin's registered tab when it is selected", function () {
    const tab = new PreferencesUIStore.TabItem({
      tabId: 'Fake Plugin',
      displayName: 'Fake plugin',
      componentClassFn: () => fake('plugin-tab'),
    });
    const before = PreferencesUIStore.selection().tabId;
    PreferencesUIStore.registerPreferencesTab(tab);
    PreferencesUIStore.switchPreferencesTab('Fake Plugin');
    try {
      const { container } = render(<PreferencesRoot />);
      expect(container.querySelector('.preferences-content .fake-plugin-tab')).not.toBe(null);
      expect(
        container.querySelector('[data-tab-id="Fake Plugin"]').getAttribute('aria-selected')
      ).toBe('true');
    } finally {
      // Unmounted first: with no tab selected ConfigPropContainer has no child to render.
      cleanup();
      PreferencesUIStore.unregisterPreferencesTab(tab);
      PreferencesUIStore.switchPreferencesTab(before);
    }
  });
});

describe('Preferences Tab order', function () {
  afterEach(cleanup);

  const tabbable = (root: Element) =>
    Array.from(
      root.querySelectorAll<HTMLElement>('button, input, select, textarea, a[href], [tabindex]')
    ).filter((el) => el.tabIndex >= 0 && !(el as HTMLButtonElement).disabled);

  it('has only the sidebar and the page as Tab stops, not the wrapper around them', function () {
    const Page = () => (
      <div>
        <button className="first-control">First</button>
        <button className="second-control">Second</button>
      </div>
    );
    Page.displayName = 'FakeTabOrderPage';
    const tab = new PreferencesUIStore.TabItem({
      tabId: 'Fake Tab Order',
      displayName: 'Fake tab order',
      componentClassFn: () => Page,
    });
    const before = PreferencesUIStore.selection().tabId;
    PreferencesUIStore.registerPreferencesTab(tab);
    PreferencesUIStore.switchPreferencesTab('Fake Tab Order');
    try {
      const { container } = render(<PreferencesRoot />);
      const wrap = container.querySelector<HTMLElement>('.preferences-wrap');
      expect(wrap.tabIndex).toBe(-1);
      const stops = tabbable(container);
      const outside = stops.filter(
        (el) => !el.closest('.preferences-sidebar, .preferences-content, .sheet-title-bar')
      );
      expect(outside).toEqual([]);
      const page = stops.filter((el) => el.closest('.preferences-content'));
      expect(page.map((el) => el.className)).toEqual(['first-control', 'second-control']);
      // The selected sidebar tab takes focus on mount, so Tab goes on from there into the page.
      expect(document.activeElement.getAttribute('data-tab-id')).toBe('Fake Tab Order');
    } finally {
      cleanup();
      PreferencesUIStore.unregisterPreferencesTab(tab);
      PreferencesUIStore.switchPreferencesTab(before);
    }
  });
});

describe('Preferences on the UI kit', function () {
  beforeEach(() => {
    // Radix Select uses delay-less timeouts, which the spec clock never fires.
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(cleanup);

  describe('Accounts', () => {
    const account = new Account({
      id: 'acct-1',
      name: 'Someone',
      emailAddress: 'someone@example.test',
      provider: 'imap',
      settings: { imap_host: 'imap.example.test' },
      aliases: [],
    } as any);

    async function renderAccounts() {
      spyOn(AccountStore, 'accounts').andReturn([account]);
      // The details pane saves on unmount.
      spyOn(Actions, 'updateAccount');
      spyOn(ipcRenderer, 'invoke').andReturn(Promise.resolve(true));
      const result = render(<PreferencesAccounts />);
      // useMailspringProfile answers after ipcRenderer.invoke resolves.
      await act(async () => {
        await Promise.resolve();
      });
      return result;
    }

    it('shows Import from Mailspring under the account list, not in the account', async () => {
      const { container } = await renderAccounts();
      const row = Array.from(container.querySelectorAll('.settings-row')).find(
        (r) => r.querySelector('.settings-row-title')?.textContent === 'Import from Mailspring'
      );
      expect(row).toBeDefined();
      expect(row.closest('.account-details')).toBe(null);
      const details = container.querySelector('.account-details');
      // The import row comes before the selected account's details.
      expect(row.compareDocumentPosition(details) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it('uses the same button variant and size as the buttons next to it', async () => {
      const { container } = await renderAccounts();
      const buttons = Array.from(container.querySelectorAll<HTMLElement>('[data-slot="button"]'));
      const importButton = buttons.find((b) => b.textContent === 'Import…');
      const rebuild = buttons.find((b) => b.textContent === 'Rebuild…');
      expect(importButton.dataset.variant).toBe('outline');
      expect(importButton.dataset.variant).toBe(rebuild.dataset.variant);
      expect(importButton.dataset.size).toBe(rebuild.dataset.size);
    });
  });

  describe('Appearance', () => {
    it('picks a theme from a kit radio group of tiles', () => {
      const setActiveTheme = spyOn(AppEnv.themes, 'setActiveTheme');
      const { container } = render(<PreferencesAppearance />);
      const group = container.querySelector('.theme-tiles');
      expect(group.getAttribute('data-slot')).toBe('radio-group');
      expect(group.getAttribute('role')).toBe('radiogroup');
      const tiles = Array.from(group.querySelectorAll<HTMLElement>('[role="radio"]'));
      const active = AppEnv.themes.getActiveThemeSetting().name;
      const checked = tiles.filter((t) => t.getAttribute('aria-checked') === 'true');
      expect(checked.length).toBe(1);
      expect(checked[0].classList.contains('on')).toBe(true);

      const other = tiles.find((t) => t.getAttribute('aria-checked') !== 'true');
      fireEvent.click(other);
      expect(setActiveTheme).toHaveBeenCalled();
      expect(setActiveTheme.mostRecentCall.args[0]).not.toBe(active);
    });

    it('picks a label colour scheme from a kit radio group of rows', () => {
      spyOn(AppEnv.config, 'set');
      const { container } = render(<PreferencesAppearance />);
      const rows = Array.from(
        container.querySelectorAll<HTMLElement>('.label-scheme-rows [role="radio"]')
      );
      expect(rows.length).toBeGreaterThan(1);
      const pastel = rows.find((r) => r.textContent === 'Pastel');
      fireEvent.click(pastel);
      expect(AppEnv.config.set).toHaveBeenCalledWith('core.appearance.labelColorScheme', 'pastel');
    });
  });

  it('writes an empty language for Automatic', () => {
    const config = { get: () => 'de', set: jasmine.createSpy('set') };
    const { container } = render(<LanguageRow config={config} />);
    const trigger = container.querySelector('[data-slot="select-trigger"]');
    expect(trigger.getAttribute('aria-label')).toBe('Language');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(selectItems().find((i) => i.textContent.startsWith('Automatic')));
    expect(config.set).toHaveBeenCalledWith('core.intl.language', '');
  });

  it('SettingsSelect maps an empty option value through Radix and back', () => {
    const onChange = jasmine.createSpy('onChange');
    const { container } = render(
      <SettingsSelect
        id="pick"
        aria-label="Service"
        value="deepl"
        onChange={onChange}
        options={[
          ['', 'None'],
          ['deepl', 'DeepL'],
        ]}
      />
    );
    const trigger = container.querySelector('#pick');
    expect(trigger.getAttribute('data-slot')).toBe('select-trigger');
    expect(trigger.textContent).toBe('DeepL');
    choose(trigger, 'None');
    expect(onChange).toHaveBeenCalledWith('');
  });

  describe('ScenarioEditorRow', () => {
    const templates = [
      new Template('from', Template.Type.String, { name: 'From' }),
      new Template('folder', Template.Type.Enum, {
        name: 'Folder',
        values: [{ value: 'f-inbox', name: 'Inbox' }],
      }),
    ];

    function renderRow(instance: any, onChange = jasmine.createSpy('onChange')) {
      const onRemove = jasmine.createSpy('onRemove');
      const onInsert = jasmine.createSpy('onInsert');
      const result = render(
        <ScenarioEditorRow
          instance={instance}
          removable
          templates={templates}
          onChange={onChange}
          onInsert={onInsert}
          onRemove={onRemove}
        />
      );
      return { ...result, onChange, onRemove, onInsert };
    }

    it('changes the template and comparator through kit selects', () => {
      const instance = templates[0].createDefaultInstance();
      const { container, onChange } = renderRow(instance);
      const [templateTrigger, comparatorTrigger] = Array.from(
        container.querySelectorAll('[data-slot="select-trigger"]')
      );
      choose(templateTrigger, 'Folder');
      expect(onChange.mostRecentCall.args[0].templateKey).toBe('folder');

      const comparators = Object.keys(templates[0].comparators);
      const second = templates[0].comparators[comparators[1]].name;
      choose(comparatorTrigger, second);
      expect(onChange.mostRecentCall.args[0].comparatorKey).toBe(comparators[1]);
    });

    it('reports an enum value with its name, and clears it with the blank option', () => {
      const { container, onChange } = renderRow({
        templateKey: 'folder',
        comparatorKey: undefined,
        value: 'f-inbox',
      });
      const triggers = Array.from(container.querySelectorAll('[data-slot="select-trigger"]'));
      const valueTrigger = triggers[triggers.length - 1];
      expect(valueTrigger.textContent).toBe('Inbox');
      choose(valueTrigger, '');
      expect(onChange.mostRecentCall.args[0].value).toBe(null);
      expect(onChange.mostRecentCall.args[0].valueName).toBeUndefined();
    });

    it('types a string value into a kit text field', () => {
      const { container, onChange } = renderRow(templates[0].createDefaultInstance());
      const input = container.querySelector('input[data-slot="input"]');
      fireEvent.change(input, { target: { value: 'ben@example.test' } });
      expect(onChange.mostRecentCall.args[0].value).toBe('ben@example.test');
    });

    it('removes and inserts rows with labelled kit buttons', () => {
      const { container, onRemove, onInsert } = renderRow(templates[0].createDefaultInstance());
      fireEvent.click(container.querySelector('[data-slot="button"][aria-label="Remove"]'));
      fireEvent.click(container.querySelector('[data-slot="button"][aria-label="Add"]'));
      expect(onRemove).toHaveBeenCalled();
      expect(onInsert).toHaveBeenCalled();
    });
  });
});
