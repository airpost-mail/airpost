import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { render as renderDOM, fireEvent, cleanup } from '@testing-library/react';
import { Actions, PreferencesUIStore } from 'mailspring-exports';
import { ConfigSelectRow, ConfigSwitchRow } from 'mailspring-component-kit';
import PreferencesSidebar from '../../../src/features/preferences/sidebar';
import { getPortalContainer } from '../../../src/components/kit/portal';

const Tab = PreferencesUIStore.TabItem;
const tab = (tabId: string, group?: any, keywords?: string[]) =>
  new Tab({ tabId, displayName: tabId, componentClassFn: () => null, group, keywords });

describe('PreferencesSidebar', function () {
  const tabs = [
    tab('Tern Server', 'server'),
    tab('General', 'app', ['startup']),
    tab('Accounts', 'mail'),
    tab('AI', 'connected', ['ollama']),
    tab('Translate It'),
    tab('Advanced', 'advanced'),
  ];
  const selection = { tabId: 'General', accountId: '' };

  const render = () =>
    TestRenderer.create(<PreferencesSidebar tabs={tabs} selection={selection} />);
  const ids = (r: TestRenderer.ReactTestRenderer) =>
    r.root
      .findAll((n) => n.props['data-tab-id'] && n.type === 'div')
      .map((n) => n.props['data-tab-id']);
  const labels = (r: TestRenderer.ReactTestRenderer) =>
    r.root
      .findAll((n) => n.props.className === 'preferences-sidebar-label')
      .map((n) => n.children.join(''));

  it('lists the server first, then each group, with ungrouped plugin tabs under From plugins', () => {
    const r = render();
    expect(ids(r)).toEqual([
      'Tern Server',
      'General',
      'Accounts',
      'AI',
      'Translate It',
      'Advanced',
    ]);
    expect(labels(r)).toEqual(['App', 'Mail', 'Connected', 'From plugins']);
  });

  it('filters by name and keyword, hides empty groups, and Escape clears the search', () => {
    const r = render();
    const input = r.root.findByType('input');
    act(() => input.props.onChange({ target: { value: 'ollama' } }));
    expect(ids(r)).toEqual(['AI']);
    expect(labels(r)).toEqual(['Connected']);

    const stopPropagation = jasmine.createSpy('stopPropagation');
    act(() => r.root.findByType('input').props.onKeyDown({ key: 'Escape', stopPropagation }));
    expect(stopPropagation).toHaveBeenCalled();
    expect(ids(r).length).toBe(tabs.length);
  });

  it('opens the first match on Enter, skipping the server entry', () => {
    spyOn(Actions, 'switchPreferencesTab');
    const r = render();
    act(() => r.root.findByType('input').props.onChange({ target: { value: 'acc' } }));
    act(() => r.root.findByType('input').props.onKeyDown({ key: 'Enter' }));
    expect(Actions.switchPreferencesTab).toHaveBeenCalledWith('Accounts');
  });
});

// Rendered with ReactDOM and driven with real events: the rows hold the UI kit's Switch and
// Select, which react-test-renderer can't click (Radix reads the event) or host (Radix keeps the
// closed list in a ReactDOM portal).
describe('Config-bound settings rows', function () {
  beforeEach(() => {
    spyOn(AppEnv.config, 'set');
    // Radix Select uses delay-less timeouts, which the spec clock never fires.
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('writes the inverse of an inverted switch', () => {
    spyOn(AppEnv.config, 'get').andReturn(true);
    const { container } = renderDOM(
      <ConfigSwitchRow keyPath="core.reading.swipeDisabled" invert label="Swipe" />
    );
    const sw = container.querySelector('[role="switch"]');
    expect(sw.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(sw);
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.reading.swipeDisabled', false);
  });

  it("sets a select's option with its own type and shows the first option when unset", () => {
    spyOn(AppEnv.config, 'get').andReturn(undefined);
    const { container } = renderDOM(
      <ConfigSelectRow
        keyPath="core.sending.undoSend"
        label="Undo Send"
        options={[
          [5000, '5 seconds'],
          [0, 'Off'],
        ]}
      />
    );
    const trigger = container.querySelector('[data-slot="select-trigger"]');
    expect(trigger.getAttribute('aria-label')).toBe('Undo Send');
    expect(trigger.textContent).toBe('5 seconds');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const off = Array.from(
      getPortalContainer().querySelectorAll<HTMLElement>('[data-slot="select-item"]')
    ).find((i) => i.textContent === 'Off');
    fireEvent.click(off);
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.sending.undoSend', 0);
  });
});
