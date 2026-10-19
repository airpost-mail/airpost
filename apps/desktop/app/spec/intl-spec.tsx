import React from 'react';
import TestRenderer from 'react-test-renderer';
import { localizedReactFragment } from '../src/intl';

const text = (node: React.ReactNode) => TestRenderer.create(<div>{node}</div>).toJSON() as any;

describe('localizedReactFragment', function () {
  it('fills %@ in order', () => {
    expect(text(localizedReactFragment('%@ and %@', 'a', 'b')).children.join('')).toBe('a and b');
  });

  it('fills %1$@ with the first argument, so translations can reorder', () => {
    expect(text(localizedReactFragment('%2$@ before %1$@', 'a', 'b')).children.join('')).toBe(
      'b before a'
    );
  });
});
