import SearchableComponentStore from '../../src/data/stores/searchable-component';

// Message frames are cross-origin, so they report their matches to the store as boxes.
describe('SearchableComponentStore with a remote region', function () {
  let element: HTMLDivElement;

  beforeEach(() => {
    // An earlier spec that unmounted an EmailFrame leaves the debounce holding a fake timer the
    // clock reset dropped; until it's cancelled the store never recalculates.
    (SearchableComponentStore as any)._recalculateMatches.cancel();
    element = document.createElement('div');
    element.style.position = 'absolute';
    element.style.top = '100px';
    document.body.appendChild(element);
  });

  afterEach(() => {
    SearchableComponentStore.unregisterSearchRegion('remote-spec');
    (SearchableComponentStore as any)._findInThread(null);
    advanceClock(100);
    element.remove();
  });

  // The store's debounce reads the real clock (underscore's own `now`), so let real time pass.
  const settle = async () => {
    await new Promise((resolve) => (window as any).originalSetTimeout(resolve, 50));
    advanceClock(100);
  };

  it('places the matches in window coordinates and tells the region which one is current', async () => {
    (SearchableComponentStore as any)._findInThread('needle');
    SearchableComponentStore.registerSearchRegion('remote-spec', {
      element,
      matches: [
        { top: 10, left: 0, height: 12, renderIndex: 0 },
        { top: 50, left: 0, height: 12, renderIndex: 1 },
      ],
    });
    await settle();
    const store = SearchableComponentStore as any;
    expect(store.matches.length).toBe(2);
    expect(store.matches[0].top).toBe(110);
    expect(SearchableComponentStore.getCurrentRegionIndex('remote-spec')).toBe(0);
    store._nextSearchResult();
    expect(SearchableComponentStore.getCurrentRegionIndex('remote-spec')).toBe(1);
    expect(SearchableComponentStore.getCurrentRegionIndex('other')).toBe(null);
  });
});
