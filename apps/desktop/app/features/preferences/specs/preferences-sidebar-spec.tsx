import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { Actions, PreferencesUIStore } from 'mailspring-exports';
import { ConfigSelectRow, ConfigSwitchRow } from 'mailspring-component-kit';
import PreferencesSidebar from '../lib/preferences-sidebar';

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

describe('Config-bound settings rows', function () {
  beforeEach(() => {
    spyOn(AppEnv.config, 'set');
  });

  it('writes the inverse of an inverted switch', () => {
    spyOn(AppEnv.config, 'get').andReturn(true);
    const r = TestRenderer.create(
      <ConfigSwitchRow keyPath="core.reading.swipeDisabled" invert label="Swipe" />
    );
    const sw = r.root.find((n) => n.props.role === 'switch');
    expect(sw.props['aria-checked']).toBe(false);
    act(() => sw.props.onClick({}));
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.reading.swipeDisabled', false);
  });

  it("sets a select's option with its own type and shows the first option when unset", () => {
    spyOn(AppEnv.config, 'get').andReturn(undefined);
    const r = TestRenderer.create(
      <ConfigSelectRow
        keyPath="core.sending.undoSend"
        label="Undo Send"
        options={[
          [5000, '5 seconds'],
          [0, 'Off'],
        ]}
      />
    );
    const select = r.root.findByType('select');
    expect(select.props.value).toBe('5000');
    act(() => select.props.onChange({ target: { value: '0' } }));
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.sending.undoSend', 0);
  });
});
