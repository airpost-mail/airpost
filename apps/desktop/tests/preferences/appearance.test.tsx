import React from 'react';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/react';

import configSchema from '../../src/config/schema';
import PreferencesAppearance from '../../src/features/preferences/tabs/appearance';
import { getPortalContainer } from '../../src/components/kit/portal';
import Package from '../../src/packages/package';

const KEY = 'core.appearance.emailRenderMode';

// Rendered with ReactDOM: the selects are the UI kit's Select, whose closed list Radix keeps in
// a ReactDOM portal that react-test-renderer can't host.
describe('PreferencesAppearance', function () {
  // The schema ConfigPropContainer passes: the `core` subtree.
  const schema = configSchema.core as any;

  beforeEach(() => {
    // Radix Select uses delay-less timeouts, which the spec clock never fires.
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('offers the email colour modes next to the theme picker', () => {
    AppEnv.config.set(KEY, 'auto');
    spyOn(AppEnv.config, 'set').andCallThrough();
    const { container } = render(<PreferencesAppearance />);
    const trigger = container.querySelector(
      '[data-slot="select-trigger"][aria-label="Message colours"]'
    );
    expect(trigger.textContent).toBe('Dark when the theme is dark');

    fireEvent.keyDown(trigger, { key: 'Enter' });
    const items = Array.from(
      getPortalContainer().querySelectorAll<HTMLElement>('[data-slot="select-item"]')
    );
    expect(items.map((i) => i.textContent)).toEqual([
      'Dark when the theme is dark',
      'Always dark',
      "Sender's colours",
    ]);

    fireEvent.click(items[2]);
    expect(AppEnv.config.set).toHaveBeenCalledWith(KEY, 'original');
  });

  it('applies the interface size when the slider is released, not while dragging', () => {
    spyOn(AppEnv.config, 'set');
    const { container } = render(<PreferencesAppearance />);
    const slider = () => container.querySelector<HTMLInputElement>('#interface-zoom-slider');

    fireEvent.change(slider(), { target: { value: '1.2' } });
    expect(AppEnv.config.set).not.toHaveBeenCalled();
    expect(slider().value).toBe('1.2');

    fireEvent.pointerUp(slider());
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.workspace.interfaceZoom', 1.2);
  });

  it("draws each theme tile in the theme's own colours", async () => {
    const { resourcePath } = AppEnv.getLoadSettings();
    const themes = ['ui-light', 'ui-dark', 'ui-darkside'].map(
      (name) => new Package(`${resourcePath}/themes/${name}`)
    );
    spyOn(AppEnv.themes, 'getAvailableThemes').andReturn(themes);
    const { container } = render(<PreferencesAppearance />);
    const miniatures = () =>
      Array.from(container.querySelectorAll<HTMLElement>('.theme-tile .theme-miniature'));
    // jasmine's expect records a failure instead of throwing, so waitFor needs a throw.
    await waitFor(() => {
      if (miniatures().some((m) => m.classList.contains('blank'))) throw new Error('blank tiles');
    });
    const colours = miniatures().map((m) => [
      m.style.background,
      m.querySelector<HTMLElement>('.sidebar').style.background,
      ...Array.from(m.querySelectorAll<HTMLElement>('.lines i')).map((i) => i.style.background),
    ]);
    expect(colours).toEqual([
      [
        'rgb(255, 255, 255)',
        'rgb(241, 243, 247)',
        'var(--system-accent, #1f70ec)',
        'rgb(35, 31, 32)',
        'rgb(35, 31, 32)',
      ],
      [
        'rgb(21, 21, 21)',
        'rgb(11, 11, 11)',
        'var(--system-accent, #4d8ff4)',
        'rgb(232, 232, 232)',
        'rgb(232, 232, 232)',
      ],
      [
        'rgb(255, 255, 255)',
        'rgb(21, 23, 29)',
        'rgb(241, 130, 96)',
        'rgb(47, 49, 55)',
        'rgb(47, 49, 55)',
      ],
    ]);
  });

  it('is not also listed under Reading', () => {
    expect(schema.properties.reading.properties.emailRenderMode).toBeUndefined();
  });
});
