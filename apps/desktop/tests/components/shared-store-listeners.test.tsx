// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import React from 'react';
import ReactDOM from 'react-dom';
import ComponentRegistry from '../../src/packages/component-registry';
import { AccountStore } from '../../src/data/stores/account';
import InjectedComponent from '../../src/components/injected/component';
import { InjectedComponentSet } from '../../src/components/injected/component-set';
import AccountColorBar from '../../src/components/account-color-bar';

const ROWS = 120;

const Icon = () => <span className="spec-icon">icon</span>;
Icon.displayName = 'SpecThreadIcon';

const listeners = (store: any) => (store._emitter ? store._emitter.listenerCount('trigger') : 0);

describe('thread-row components and store listeners', function () {
  let node: HTMLDivElement = null;

  beforeEach(function () {
    node = document.createElement('div');
  });

  afterEach(function () {
    ReactDOM.unmountComponentAtNode(node);
    ComponentRegistry.unregister(Icon);
  });

  const renderRows = () =>
    ReactDOM.render(
      <div>
        {Array.from({ length: ROWS }, (_, i) => (
          <div key={i}>
            <InjectedComponentSet
              containersRequired={false}
              matching={{ role: 'SpecThreadIcon' }}
              exposedProps={{ i }}
            />
            <InjectedComponent
              matching={{ role: 'SpecThreadTimestamp' }}
              fallback={() => <span />}
              exposedProps={{ i }}
            />
            <AccountColorBar accountId={TEST_ACCOUNT_ID} />
          </div>
        ))}
      </div>,
      node
    );

  it('share one ComponentRegistry and one AccountStore listener however many rows mount', function () {
    const registryBefore = listeners(ComponentRegistry);
    const accountsBefore = listeners(AccountStore);
    renderRows();
    expect(listeners(ComponentRegistry) - registryBefore).toBe(1);
    expect(listeners(AccountStore) - accountsBefore).toBe(1);
    ReactDOM.unmountComponentAtNode(node);
    expect(listeners(ComponentRegistry)).toBe(registryBefore);
    expect(listeners(AccountStore)).toBe(accountsBefore);
  });

  it('still shows a component registered after the rows mounted', function () {
    renderRows();
    expect(node.querySelectorAll('.spec-icon').length).toBe(0);
    ComponentRegistry.register(Icon, { role: 'SpecThreadIcon' });
    // register() triggers through a debounce on the real clock.
    ComponentRegistry.trigger(ComponentRegistry);
    expect(node.querySelectorAll('.spec-icon').length).toBe(ROWS);
  });
});
