import React from 'react';
import ReactDOM from 'react-dom';
import { Rx, ComponentRegistry, DraftStore } from 'mailspring-exports';
import DraftPreview from '../../../src/features/draft-list/draft-preview';
import DraftListStore from '../../../src/features/draft-list/draft-list-store';

let lastComposerProps = null;
let composerFocused = 0;

class FakeComposer extends React.Component<Record<string, unknown>> {
  static displayName = 'FakeComposer';
  focus() {
    composerFocused += 1;
  }
  render() {
    lastComposerProps = this.props;
    return <div className="fake-composer" />;
  }
}

describe('DraftPreview', function () {
  let container: HTMLDivElement;

  const render = ({ headerMessageId = null, selected = [] as any[], sending = false }) => {
    spyOn(DraftListStore, 'focusedHeaderMessageId').andReturn(headerMessageId);
    spyOn(DraftListStore, 'selectionObservable').andReturn(Rx.Observable.just(selected));
    spyOn(DraftStore, 'isSendingDraft').andReturn(sending);
    ReactDOM.render(<DraftPreview />, container);
  };

  beforeEach(() => {
    lastComposerProps = null;
    composerFocused = 0;
    container = document.createElement('div');
    ComponentRegistry.register(FakeComposer, { role: 'Composer' });
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    ComponentRegistry.unregister(FakeComposer);
  });

  it('renders an empty pane when no draft is focused', () => {
    render({});
    expect(container.querySelector('.draft-preview-empty')).not.toBe(null);
    expect(container.querySelector('.fake-composer')).toBe(null);
  });

  it('renders the focused draft in a composer that keeps it and does not take focus', () => {
    render({ headerMessageId: 'hmid-1' });
    expect(container.querySelector('.fake-composer')).not.toBe(null);
    expect(lastComposerProps.headerMessageId).toBe('hmid-1');
    expect(lastComposerProps.keepPristineDraft).toBe(true);
    expect(lastComposerProps.skipInitialFocus).toBe(true);
  });

  it('shows the stack when several drafts are selected', () => {
    render({ headerMessageId: null, selected: [{ id: 'a' }, { id: 'b' }] });
    expect(container.querySelector('.selected-items-stack .count').textContent).toBe('2');
  });

  it('shows the sending state instead of the composer', () => {
    render({ headerMessageId: 'hmid-1', sending: true });
    expect(container.querySelector('.fake-composer')).toBe(null);
    expect(container.querySelector('.draft-preview-status')).not.toBe(null);
  });

  it('moves focus into the composer when asked by the list', () => {
    render({ headerMessageId: 'hmid-1' });
    DraftListStore._focusedHeaderMessageId = 'hmid-1';
    expect(DraftListStore.focusPreview()).toBe(true);
    expect(composerFocused).toBe(1);
    DraftListStore._focusedHeaderMessageId = null;
  });
});
