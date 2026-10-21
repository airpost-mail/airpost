// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import * as Actions from '../../src/data/actions';
import ComponentRegistry from '../../src/packages/component-registry';
import { Contact } from '../../src/data/models/contact';
import { DestroyContactTask } from '../../src/data/tasks/destroy-contact';
import WorkspaceStore from '../../src/data/stores/workspace';
import { ListTabular } from '../../src/components/list/tabular';
import { InjectedComponentSet } from '../../src/components/injected/component-set';
import Sheet from '../../src/components/sheets/sheet';
import { Store } from '../../src/features/contacts/store';
import { ContactDetailToolbar } from '../../src/features/contacts/detail-toolbar';
import { AddContactToolbar } from '../../src/features/contacts/add-contact-toolbar';
import { ContactList, ContactListSearch } from '../../src/features/contacts/list';
import { ContactDetailEdit } from '../../src/features/contacts/detail-edit';
import { ListEditor } from '../../src/features/contacts/list-editor';
import { YYMMDDInput } from '../../src/features/contacts/yymmdd-input';
import { ContactBase } from '../../src/features/contacts/types';
import * as GoogleSupport from '../../src/features/contacts/google-support';

// The exported components read their props from the store.
const DetailToolbar = ContactDetailToolbar as React.ComponentType;
const AddToolbar = AddContactToolbar as React.ComponentType;

const contactFor = (id: string) =>
  new Contact({
    id,
    accountId: TEST_ACCOUNT_ID,
    source: 'carddav',
    name: `Contact ${id}`,
    email: `${id}@example.com`,
    contactGroups: [],
  });

const emptyData = (): ContactBase => ({
  name: {
    displayName: '',
    familyName: '',
    givenName: 'Ada',
    honorificPrefix: '',
    honorificSuffix: '',
  },
  company: '',
  title: '',
  emailAddresses: [],
  phoneNumbers: [],
  addresses: [],
  urls: [],
  nicknames: [],
  relations: [],
  birthdays: [],
  notes: '',
});

describe('Contacts on the UI kit', function () {
  let listSource: InstanceType<typeof ListTabular.DataSource.DumbArrayDataSource>;

  beforeEach(() => {
    spyOn(GoogleSupport, 'showGPeopleReadonlyNotice').andReturn(false);
    listSource = new ListTabular.DataSource.DumbArrayDataSource<Contact>();
    spyOn(Store, 'listSource').andCallFake(() => listSource);
    spyOn(Store, 'editing').andReturn(false);
  });

  afterEach(() => cleanup());

  describe('ContactDetailToolbar', () => {
    beforeEach(() => {
      spyOn(Store, 'perspective').andReturn({
        type: 'group',
        accountId: TEST_ACCOUNT_ID,
        groupId: 'g1',
        label: 'Friends',
      });
    });

    it('renders kit toolbar buttons, disabled with nothing selected', () => {
      const { container, getByText } = render(<DetailToolbar />);
      const remove = getByText('Remove from Group').closest('button');
      const edit = getByText('Edit').closest('button');
      const del = container.querySelector('[aria-label="Delete"]');
      const exp = container.querySelector('[aria-label="Export vCard"]');

      for (const b of [remove, edit]) {
        expect(b.getAttribute('data-slot')).toBe('button');
        expect(b.getAttribute('data-variant')).toBe('toolbar');
        expect(b.classList.contains('btn-toolbar')).toBe(true);
      }
      for (const b of [del, exp]) {
        expect(b.getAttribute('data-slot')).toBe('icon-button');
        expect(b.classList.contains('btn-toolbar')).toBe(true);
      }
      for (const b of [remove, edit, del, exp]) {
        expect((b as HTMLButtonElement).disabled).toBe(true);
        expect(b.getAttribute('tabindex')).toBe('-1');
      }
    });

    it('acts on the selected contact', () => {
      const contact = contactFor('c1');
      listSource.setItems([contact]);
      listSource.selection.set([contact]);
      spyOn(Store, 'setEditing');
      const queueTask = spyOn(Actions, 'queueTask');

      const { container, getByText } = render(<DetailToolbar />);
      const edit = getByText('Edit').closest('button');
      expect(edit.disabled).toBe(false);
      fireEvent.click(edit);
      expect(Store.setEditing).toHaveBeenCalledWith('c1');

      fireEvent.click(container.querySelector('[aria-label="Delete"]'));
      expect(queueTask.callCount).toBe(1);
      expect(queueTask.calls[0].args[0] instanceof DestroyContactTask).toBe(true);
    });
  });

  describe('AddContactToolbar', () => {
    it('is disabled in the unified view and keeps its explanation as the tooltip trigger', () => {
      spyOn(Store, 'perspective').andReturn({ type: 'unified' });
      const { container } = render(<AddToolbar />);
      const button = container.querySelector<HTMLButtonElement>('button.btn-new-contact');
      expect(button.getAttribute('data-slot')).toBe('button');
      expect(button.disabled).toBe(true);
      expect(button.getAttribute('aria-label')).toBe('Select an account to add a contact.');
      // The wrapper span is the trigger, since the disabled button takes no pointer events.
      const trigger = button.parentElement;
      expect(trigger.getAttribute('data-slot')).toBe('tooltip-trigger');
    });

    it('starts a new contact for the selected account', () => {
      spyOn(Store, 'perspective').andReturn({
        type: 'all',
        accountId: TEST_ACCOUNT_ID,
        label: 'All Contacts',
      });
      spyOn(Store, 'setEditing');
      const { container } = render(<AddToolbar />);
      const button = container.querySelector<HTMLButtonElement>('button.btn-new-contact');
      expect(button.disabled).toBe(false);
      fireEvent.click(button);
      expect(Store.setEditing).toHaveBeenCalledWith('new');
    });
  });

  describe('ContactList', () => {
    it('renders the Found in Mail bar itself, with a kit button', () => {
      spyOn(Store, 'perspective').andReturn({
        type: 'found-in-mail',
        accountId: TEST_ACCOUNT_ID,
        label: 'Found in Mail',
      });
      AppEnv.config.set('core.contacts.findInMailDisabled', []);
      const { container } = render(<ContactList />);
      const bar = container.querySelector('.contact-list-drop-target > .found-in-mail-enabled-bar');
      expect(bar).not.toBeNull();
      const button = bar.querySelector('[data-slot="button"]');
      expect(button.textContent).toBe('Disable');
      fireEvent.click(button);
      expect(AppEnv.config.get('core.contacts.findInMailDisabled')).toEqual([TEST_ACCOUNT_ID]);
    });

    it('shows no bar for other perspectives', () => {
      spyOn(Store, 'perspective').andReturn({ type: 'unified' });
      const { container } = render(<ContactList />);
      expect(container.querySelector('.found-in-mail-enabled-bar')).toBeNull();
    });
  });

  describe('ContactListSearch', () => {
    it('uses a TextField and a clear IconButton', () => {
      spyOn(Store, 'perspective').andReturn({ type: 'unified' });
      spyOn(Store, 'search').andReturn('ada');
      spyOn(Store, 'setSearch');
      const { container } = render(<ContactListSearch />);
      const input = container.querySelector('.contact-search input');
      expect(input.getAttribute('data-slot')).toBe('input');
      const clear = container.querySelector('.search-accessory.clear');
      expect(clear.getAttribute('data-slot')).toBe('icon-button');
      expect(clear.getAttribute('aria-label')).toBe('Clear search');
      expect(fireEvent.mouseDown(clear)).toBe(false); // focus stays in the field
      fireEvent.click(clear);
      expect(Store.setSearch).toHaveBeenCalledWith('');
    });

    it('has no clear button with an empty search', () => {
      spyOn(Store, 'perspective').andReturn({ type: 'unified' });
      spyOn(Store, 'search').andReturn('');
      const { container } = render(<ContactListSearch />);
      expect(container.querySelector('.search-accessory.clear')).toBeNull();
    });
  });

  describe('ContactDetailEdit', () => {
    it('labels each kit field', () => {
      const onChange = jasmine.createSpy('onChange');
      const data = emptyData();
      const { getByLabelText } = render(
        <ContactDetailEdit data={data} contact={contactFor('c1')} onChange={onChange} />
      );
      const first = getByLabelText('First Name') as HTMLInputElement;
      expect(first.getAttribute('data-slot')).toBe('input');
      expect(first.value).toBe('Ada');
      fireEvent.change(first, { target: { value: 'Grace' } });
      expect(onChange).toHaveBeenCalledWith({ name: { ...data.name, givenName: 'Grace' } });

      const notes = getByLabelText('Notes');
      expect(notes.getAttribute('data-slot')).toBe('textarea');
      expect(notes.classList.contains('contact-notes-textarea')).toBe(true);
    });
  });

  describe('ListEditor', () => {
    it('adds and removes rows with kit buttons', () => {
      const onChange = jasmine.createSpy('onChange');
      const { container } = render(
        <ListEditor<{ value: string }>
          items={[{ value: 'a' }]}
          itemTemplate={{ value: '' }}
          onChange={onChange}
        >
          {(item) => <span>{item.value}</span>}
        </ListEditor>
      );
      const remove = container.querySelector('button.remove');
      const add = container.querySelector('button.add');
      expect(remove.getAttribute('data-slot')).toBe('button');
      expect(remove.getAttribute('aria-label')).toBe('Remove');
      expect(add.getAttribute('aria-label')).toBe('Add');
      fireEvent.click(add);
      expect(onChange).toHaveBeenCalledWith([{ value: 'a' }, { value: '' }]);
      fireEvent.click(remove);
      expect(onChange).toHaveBeenCalledWith([]);
    });
  });

  describe('YYMMDDInput', () => {
    it('reads the three number fields on blur', () => {
      const onChange = jasmine.createSpy('onChange');
      const { getByLabelText } = render(
        <YYMMDDInput value={{ year: 1990, month: 4, day: 2 }} onChange={onChange} />
      );
      const year = getByLabelText('Year');
      expect(year.getAttribute('data-slot')).toBe('input');
      expect(year.getAttribute('type')).toBe('number');
      fireEvent.blur(year);
      expect(onChange).toHaveBeenCalledWith({ year: 1990, month: 4, day: 2 });
    });
  });
});

// The Contacts sheet's columns and toolbars are registry locations that the Sheet and Toolbar
// read; no plugin uses them.
describe('Contacts sheet slots', function () {
  afterEach(() => cleanup());

  it('renders components registered in the three column locations and their toolbars', () => {
    const mode = WorkspaceStore.layoutMode();
    const columns = ['ContactsSidebar', 'ContactsList', 'ContactsDetail'];
    WorkspaceStore.defineSheet('ContactsSlotSpec', {}, { [mode]: columns });
    const fakes = columns.flatMap((col) => {
      const Fake = () => <div className={`fake-${col}`} />;
      Fake.displayName = `Fake${col}`;
      const FakeToolbar = () => <div className={`fake-${col}-toolbar`} />;
      FakeToolbar.displayName = `Fake${col}Toolbar`;
      ComponentRegistry.register(Fake, { location: WorkspaceStore.Location[col] });
      ComponentRegistry.register(FakeToolbar, {
        location: WorkspaceStore.Location[col].Toolbar,
      });
      return [Fake, FakeToolbar];
    });
    try {
      const { container } = render(
        <Sheet data={WorkspaceStore.Sheet.ContactsSlotSpec} depth={0} />
      );
      for (const col of columns) {
        expect(container.querySelector(`.column-${col} .fake-${col}`)).not.toBeNull();
        const toolbar = render(
          <InjectedComponentSet matching={{ location: WorkspaceStore.Location[col].Toolbar }} />
        );
        expect(toolbar.container.querySelector(`.fake-${col}-toolbar`)).not.toBeNull();
      }
    } finally {
      fakes.forEach((f) => ComponentRegistry.unregister(f));
      WorkspaceStore.undefineSheet('ContactsSlotSpec');
    }
  });
});
