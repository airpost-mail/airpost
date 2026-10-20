import React from 'react';
import TestRenderer from 'react-test-renderer';
import ThreadSharingButton from '../lib/button';

describe('ThreadSharingButton', () => {
  it('is disabled with the capability store reason', () => {
    const renderer = TestRenderer.create(<ThreadSharingButton items={[]} thread={null} />);
    const button = renderer.root.findByType('button');
    expect(button.props.disabled).toBe(true);
    expect(button.props.title).toBe('Sharing threads: Needs a Tern server');
    renderer.unmount();
  });
});
