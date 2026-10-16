import React from 'react';
import TestRenderer from 'react-test-renderer';
import ShareButton from '../lib/dashboard/share-button';

describe('Activity ShareButton', () => {
  it('is disabled with the capability store reason', () => {
    const renderer = TestRenderer.create(<ShareButton />);
    const button = renderer.root.findByProps({ 'aria-disabled': 'true' });
    expect(button.props.title).toBe('Sharing reports: Needs a Tern server');
    renderer.unmount();
  });
});
