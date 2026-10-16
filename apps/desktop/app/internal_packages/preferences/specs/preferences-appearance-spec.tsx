import React from 'react';
import TestRenderer from 'react-test-renderer';

import configSchema from '../../../src/config-schema';
import PreferencesAppearance from '../lib/tabs/preferences-appearance';

const KEY = 'core.appearance.emailRenderMode';

describe('PreferencesAppearance', function () {
  // The schema ConfigPropContainer passes: the `core` subtree.
  const schema = configSchema.core as any;

  const render = (values: { [key: string]: unknown }) => {
    const config = {
      get: (key: string) => values[key] as string,
      set: jasmine.createSpy('set'),
      toggle: jasmine.createSpy('toggle'),
    };
    const renderer = TestRenderer.create(
      <PreferencesAppearance config={config} configSchema={schema} />
    );
    return { renderer, config };
  };

  it('offers the email colour modes next to the theme picker', () => {
    const { renderer, config } = render({ [KEY]: 'auto' });
    const select = renderer.root.find((node) => node.type === 'select' && node.props.id === KEY);
    expect(select.props.value).toBe('auto');
    expect(select.findAllByType('option').map((o) => o.props.value)).toEqual([
      'auto',
      'dark',
      'original',
    ]);

    select.props.onChange({ target: { value: 'original', blur: () => {} } });
    expect(config.set).toHaveBeenCalledWith(KEY, 'original');
  });

  it('is not also listed under Reading', () => {
    expect(schema.properties.reading.properties.emailRenderMode).toBeUndefined();
  });
});
