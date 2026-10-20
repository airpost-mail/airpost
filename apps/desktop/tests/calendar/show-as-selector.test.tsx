import React from 'react';
import ReactDOM from 'react-dom';
import { fireEvent } from '@testing-library/react';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait } from '../support/kit-helpers';
import {
  ShowAsSelector,
  defaultShowAs,
} from '../../src/features/main-calendar/event/show-as-selector';

describe('defaultShowAs', function () {
  it('starts an all-day event Free and a timed one Busy', function () {
    expect(defaultShowAs(true)).toBe('TRANSPARENT');
    expect(defaultShowAs(false)).toBe('OPAQUE');
  });
});

describe('ShowAsSelector', function () {
  let host: HTMLDivElement;

  beforeEach(function () {
    host = document.createElement('div');
  });
  afterEach(function () {
    ReactDOM.unmountComponentAtNode(host);
  });

  it('offers Busy and Free, as the two TRANSP values', async function () {
    jasmine.unspy(window, 'setTimeout');
    const onChange = jasmine.createSpy('onChange');
    document.body.appendChild(host);
    ReactDOM.render(<ShowAsSelector value="OPAQUE" onChange={onChange} />, host);
    const trigger = host.querySelector('[data-slot="select-trigger"]') as HTMLElement;
    expect(trigger.textContent).toBe('Busy');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    await wait(50);
    const items = Array.from(
      getPortalContainer().querySelectorAll('[data-slot="select-item"]')
    ) as HTMLElement[];
    expect(items.map((i) => i.textContent)).toEqual(['Busy', 'Free']);
    fireEvent.keyDown(items[1], { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith('TRANSPARENT');
    host.remove();
  });
});
