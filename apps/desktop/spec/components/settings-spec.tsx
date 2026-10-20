import React from 'react';
import ReactDOM from 'react-dom';
import { act } from 'react-dom/test-utils';
import { render as renderDOM, fireEvent, cleanup } from '@testing-library/react';
import {
  SegmentedControl,
  ConfigSelectRow,
  ProviderCard,
  SettingsSwitch,
} from '../../src/components/settings';
import { getPortalContainer } from '../../src/components/kit/portal';

type Kind = 'all' | 'open' | 'click';

const OPTIONS: [Kind, string][] = [
  ['all', 'All'],
  ['open', 'Opens'],
  ['click', 'Clicks'],
];

function Controlled(props: { initial: Kind | null; onChange?: (v: Kind) => void }) {
  const [value, setValue] = React.useState<Kind | null>(props.initial);
  return (
    <SegmentedControl<Kind>
      label="Kind"
      value={value}
      options={OPTIONS}
      onChange={(v) => {
        setValue(v);
        props.onChange?.(v);
      }}
    />
  );
}

describe('SegmentedControl', function segmentedControlSpec() {
  let container: HTMLDivElement;

  const segments = () =>
    Array.from(container.querySelectorAll<HTMLButtonElement>('[role="radio"]'));
  const checked = () => segments().find((b) => b.getAttribute('aria-checked') === 'true');
  const press = (target: Element, key: string) =>
    act(() => {
      target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
    });
  const render = (el: React.ReactElement) =>
    act(() => {
      ReactDOM.render(el, container);
    });

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    container.remove();
  });

  it('is a labelled radio group whose chosen segment is the only tab stop', () => {
    render(<Controlled initial="open" />);
    const group = container.querySelector('[role="radiogroup"]');
    expect(group.getAttribute('aria-label')).toBe('Kind');
    expect(segments().map((b) => b.tabIndex)).toEqual([-1, 0, -1]);
    expect(checked().textContent).toBe('Opens');
  });

  it('moves the choice and the focus with the arrow keys, wrapping at the ends', () => {
    render(<Controlled initial="open" />);
    segments()[1].focus();

    press(document.activeElement, 'ArrowRight');
    expect(checked().textContent).toBe('Clicks');
    expect(document.activeElement).toBe(checked());
    expect(segments().map((b) => b.tabIndex)).toEqual([-1, -1, 0]);

    press(document.activeElement, 'ArrowRight');
    expect(checked().textContent).toBe('All');
    expect(document.activeElement).toBe(checked());

    press(document.activeElement, 'ArrowLeft');
    expect(checked().textContent).toBe('Clicks');
    expect(document.activeElement).toBe(checked());
  });

  it('ignores other keys and leaves their default alone', () => {
    const onChange = jasmine.createSpy('onChange');
    render(<Controlled initial="all" onChange={onChange} />);
    const event = new KeyboardEvent('keydown', { key: 'a', bubbles: true, cancelable: true });
    act(() => {
      segments()[0].dispatchEvent(event);
    });
    expect(onChange).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it('gives the tab stop to the first segment when nothing is chosen', () => {
    render(<Controlled initial={null} />);
    expect(segments().map((b) => b.tabIndex)).toEqual([0, -1, -1]);
    press(segments()[0], 'ArrowRight');
    expect(checked().textContent).toBe('All');
  });

  it('does nothing on arrow keys when disabled', () => {
    const onChange = jasmine.createSpy('onChange');
    render(<SegmentedControl<Kind> value="all" options={OPTIONS} onChange={onChange} disabled />);
    press(container.querySelector('[role="radiogroup"]'), 'ArrowRight');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('takes a className for restyling and still clicks', () => {
    const onChange = jasmine.createSpy('onChange');
    render(
      <SegmentedControl<Kind>
        className="segmented-control"
        value="all"
        options={OPTIONS}
        onChange={onChange}
      />
    );
    const group = container.querySelector('[role="radiogroup"]');
    expect(group.className).toBe('settings-segmented segmented-control');
    act(() => {
      segments()[2].click();
    });
    expect(onChange).toHaveBeenCalledWith('click');
  });
});

describe('ConfigSelectRow on the UI kit', function configSelectRowSpec() {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
    spyOn(AppEnv.config, 'set');
  });

  afterEach(() => cleanup());

  const renderRow = (value: string) => {
    spyOn(AppEnv.config, 'get').andReturn(value);
    return renderDOM(
      <ConfigSelectRow
        keyPath="core.some.key"
        label="Choice"
        options={[
          ['', 'None'],
          ['a', 'A'],
        ]}
      />
    ).container.querySelector('[data-slot="select-trigger"]');
  };
  const pick = (trigger: Element, label: string) => {
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const item = Array.from(
      getPortalContainer().querySelectorAll<HTMLElement>('[data-slot="select-item"]')
    ).find((i) => i.textContent === label);
    fireEvent.click(item);
  };

  it('shows and sets an empty option value, which Radix keeps for "cleared"', () => {
    const trigger = renderRow('');
    expect(trigger.textContent).toBe('None');
    pick(trigger, 'A');
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.some.key', 'a');
  });

  it('sets the empty value back', () => {
    const trigger = renderRow('a');
    expect(trigger.textContent).toBe('A');
    pick(trigger, 'None');
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.some.key', '');
  });
});

describe('ProviderCard', function providerCardSpec() {
  afterEach(() => cleanup());

  it('leaves Enter and Space on a control in its head to the control', () => {
    const onToggle = jasmine.createSpy('onToggle');
    const { container } = renderDOM(
      <ProviderCard
        title="Provider"
        onToggle={onToggle}
        actions={<SettingsSwitch checked={false} onChange={() => {}} label="On" />}
      >
        <div>Fields</div>
      </ProviderCard>
    );
    const sw = container.querySelector('[role="switch"]');
    const onSwitch = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true });
    sw.dispatchEvent(onSwitch);
    expect(onSwitch.defaultPrevented).toBe(false);
    expect(onToggle).not.toHaveBeenCalled();

    const head = container.querySelector('.settings-provider-head');
    const onHead = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    head.dispatchEvent(onHead);
    expect(onHead.defaultPrevented).toBe(true);
    expect(onToggle).toHaveBeenCalledWith(true);
  });
});
