// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import { ComponentRegistry, Contact, Message, MessageWithEditorState } from 'mailspring-exports';
import { ComposerSupport } from 'mailspring-component-kit';
import ComposerView from '../../src/features/composer/view';

// The composer's plugin slots, each rendered with a fake registered component. The
// signature picker in the From row is built in and renders without a registration.

function fake(name: string) {
  const Component = (props: { headerMessageId: string; draft: Message }) => (
    <span className={`fake-${name}`} data-header-message-id={props.headerMessageId} />
  );
  Component.displayName = `Fake${name}`;
  return Component;
}

const FakeActionButton = fake('action-button');
const FakeFooter = fake('footer');
const FakeWorkspace = fake('workspace');
const FakeFromField = fake('from-field');

describe('Composer slots', function () {
  let draft: MessageWithEditorState;
  let session: any;

  beforeEach(() => {
    draft = new Message({
      draft: true,
      headerMessageId: 'slots-draft',
      accountId: TEST_ACCOUNT_ID,
      from: [new Contact({ email: TEST_ACCOUNT_EMAIL, name: TEST_ACCOUNT_NAME })],
      to: [],
      cc: [],
      bcc: [],
      subject: 'Hello',
      body: '<p>Hi</p>',
      date: new Date(0),
      files: [],
    }) as MessageWithEditorState;
    draft.bodyEditorState = ComposerSupport.Conversion.convertFromHTML(draft.body);
    session = {
      draft: () => draft,
      changes: { add: jasmine.createSpy('changes.add') },
      setMountedEditor: () => {},
      ensureCorrectAccount: () => {},
      listen: () => () => {},
    };
  });

  afterEach(() => {
    cleanup();
    for (const c of [FakeActionButton, FakeFooter, FakeWorkspace, FakeFromField]) {
      ComponentRegistry.unregister(c);
    }
  });

  const renderComposer = () => {
    const { container } = render(<ComposerView draft={draft} session={session} />);
    // The slots are deferred: they fill in 400ms after mounting.
    advanceClock(500);
    return container;
  };

  it('renders Composer:ActionButton components in the action bar', () => {
    ComponentRegistry.register(FakeActionButton, { role: 'Composer:ActionButton' });
    const container = renderComposer();
    const el = container.querySelector('.composer-action-bar-plugins .fake-action-button');
    expect(el).not.toBe(null);
    expect(el.getAttribute('data-header-message-id')).toBe('slots-draft');
  });

  it('renders Composer:Footer components below the body', () => {
    ComponentRegistry.register(FakeFooter, { role: 'Composer:Footer' });
    const container = renderComposer();
    const el = container.querySelector('.composer-footer-region .fake-footer');
    expect(el).not.toBe(null);
    expect(el.getAttribute('data-header-message-id')).toBe('slots-draft');
  });

  it('renders Composer:ActionBarWorkspace components above the action bar', () => {
    ComponentRegistry.register(FakeWorkspace, { role: 'Composer:ActionBarWorkspace' });
    const container = renderComposer();
    const el = container.querySelector('.composer-action-bar-workspace-wrap .fake-workspace');
    expect(el).not.toBe(null);
    expect(el.getAttribute('data-header-message-id')).toBe('slots-draft');
  });

  it('renders Composer:FromFieldComponents components before the built-in signature picker', () => {
    ComponentRegistry.register(FakeFromField, { role: 'Composer:FromFieldComponents' });
    const container = renderComposer();
    const region = container.querySelector('.from-field .dropdown-component');
    expect(region).not.toBe(null);
    const fakeEl = region.querySelector('.fake-from-field');
    const signature = region.querySelector('.signature-button-dropdown');
    expect(fakeEl).not.toBe(null);
    expect(signature).not.toBe(null);
    expect(
      fakeEl.compareDocumentPosition(signature) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
  });

  it('renders the signature picker in the From row with nothing registered', () => {
    const container = renderComposer();
    expect(container.querySelector('.from-field .signature-button-dropdown')).not.toBe(null);
  });

  it('renders the attach and delete buttons as kit icon buttons', () => {
    const container = renderComposer();
    for (const cls of ['btn-attach', 'btn-trash']) {
      const button = container.querySelector(`.composer-action-bar-content .${cls}`);
      expect(button.getAttribute('data-slot')).toBe('icon-button');
      expect(button.classList.contains('btn-toolbar')).toBe(true);
      expect(button.getAttribute('aria-label')).toBeTruthy();
      expect(button.hasAttribute('title')).toBe(false);
    }
  });
});
