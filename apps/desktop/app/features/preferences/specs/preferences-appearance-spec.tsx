import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';

import configSchema from '../../../src/config-schema';
import PreferencesAppearance from '../lib/tabs/preferences-appearance';

const KEY = 'core.appearance.emailRenderMode';

describe('PreferencesAppearance', function () {
  // The schema ConfigPropContainer passes: the `core` subtree.
  const schema = configSchema.core as any;

  it('offers the email colour modes next to the theme picker', () => {
    AppEnv.config.set(KEY, 'auto');
    spyOn(AppEnv.config, 'set').andCallThrough();
    const renderer = TestRenderer.create(<PreferencesAppearance />);
    const select = renderer.root.find(
      (node) => node.type === 'select' && node.props['aria-label'] === 'Message colours'
    );
    expect(select.props.value).toBe('auto');
    expect(select.findAllByType('option').map((o) => o.props.value)).toEqual([
      'auto',
      'dark',
      'original',
    ]);

    select.props.onChange({ target: { value: 'original' } });
    expect(AppEnv.config.set).toHaveBeenCalledWith(KEY, 'original');
    renderer.unmount();
  });

  it('applies the interface size when the slider is released, not while dragging', () => {
    spyOn(AppEnv.config, 'set');
    const renderer = TestRenderer.create(<PreferencesAppearance />);
    const slider = () => renderer.root.find((node) => node.props.id === 'interface-zoom-slider');

    act(() => slider().props.onChange({ target: { value: '1.2' } }));
    expect(AppEnv.config.set).not.toHaveBeenCalled();
    expect(slider().props.value).toBe(1.2);

    act(() => slider().props.onPointerUp());
    expect(AppEnv.config.set).toHaveBeenCalledWith('core.workspace.interfaceZoom', 1.2);
    renderer.unmount();
  });

  it('is not also listed under Reading', () => {
    expect(schema.properties.reading.properties.emailRenderMode).toBeUndefined();
  });
});
