// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup, act } from '@testing-library/react';
import { ScrollRegion } from '../../src/components/scroll-region';

describe('ScrollRegion scrollbar', () => {
  afterEach(() => cleanup());

  const trackVisibility = (totalHeight: number, viewportHeight: number) => {
    const ref = React.createRef<ScrollRegion>();
    const { container } = render(
      <ScrollRegion ref={ref} style={{ height: 400 }}>
        <div />
      </ScrollRegion>
    );
    act(() => ref.current!._setSharedState({ totalHeight, viewportHeight }));
    return (container.querySelector('.scrollbar-track') as HTMLElement).style.visibility;
  };

  it('is hidden when the content is shorter than the viewport', () => {
    expect(trackVisibility(300, 400)).toBe('hidden');
  });

  it('is hidden when the content fits a fractional viewport height', () => {
    expect(trackVisibility(401, 400.4)).toBe('hidden');
  });

  it('shows when the content overflows', () => {
    expect(trackVisibility(800, 400)).toBe('');
  });
});
