import SheetContainer from '../../../src/components/sheets/sheet-container';
import { WorkspaceStore } from 'mailspring-exports';
import { SHOW_NAV_RAIL_CONFIG_KEY } from '../../../src/components/nav-rail';

describe('SheetContainer', () => {
  it('re-reads the window load settings when a hot window receives its window props', () => {
    let onWindowPropsReceived: () => void;
    const dispose = jasmine.createSpy('dispose');

    spyOn(AppEnv, 'onWindowPropsReceived').andCallFake((callback) => {
      onWindowPropsReceived = callback as () => void;
      return { dispose };
    });
    spyOn(WorkspaceStore, 'listen').andReturn(() => {});

    const container = new SheetContainer({});
    spyOn(container, 'setState');
    container.componentDidMount();

    onWindowPropsReceived();
    expect(container.setState).toHaveBeenCalled();

    container.componentWillUnmount();
    expect(dispose).toHaveBeenCalled();
  });
});

describe('SheetContainer toolbar layout', () => {
  const threads = { id: 'Threads' } as any;
  const preferences = { id: 'Preferences', hideToolbar: true } as any;
  const realPreferences = WorkspaceStore.Sheet.Preferences;
  // WorkspaceStore pushes and pops this one array in place.
  let stack: any[];
  let container: SheetContainer;
  let toolbars: Array<{ recomputeLayout: jasmine.Spy }>;

  beforeEach(() => {
    WorkspaceStore.Sheet.Preferences = preferences;
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    spyOn(AppEnv, 'onWindowPropsReceived').andReturn({ dispose: () => {} });
    spyOn(WorkspaceStore, 'listen').andReturn(() => {});
    spyOn(window, 'requestAnimationFrame').andCallFake((fn: FrameRequestCallback) => {
      fn(0);
      return 0;
    });
    stack = [threads];
    spyOn(WorkspaceStore, 'sheetStack').andCallFake(() => stack);
    container = new SheetContainer({});
    // Not mounted: update() below sets the state the way a re-render would.
    spyOn(container, 'setState');
    container.componentDidMount();
    toolbars = [0, 1].map(() => ({ recomputeLayout: jasmine.createSpy('recomputeLayout') }));
    container._toolbarComponents = { 0: toolbars[0], 1: toolbars[1], 2: null };
  });

  afterEach(() => {
    container.componentWillUnmount();
    WorkspaceStore.Sheet.Preferences = realPreferences;
  });

  const update = (change: () => void) => {
    change();
    container.state = container._getStateFromStores();
    container.componentDidUpdate();
  };
  const calls = () => toolbars[0].recomputeLayout.callCount;

  // A resize under Preferences measures the columns with no rail; the root toolbar has to be
  // measured again when the rail comes back, or its items sit a rail's width off.
  it('remeasures the toolbars when Preferences opens and closes', () => {
    update(() => stack.push(preferences));
    expect(calls()).toBe(1);
    update(() => stack.pop());
    expect(calls()).toBe(2);
    expect(toolbars[1].recomputeLayout.callCount).toBe(2);
  });

  it('remeasures when the rail is hidden or shown', () => {
    update(() => AppEnv.config.set(SHOW_NAV_RAIL_CONFIG_KEY, false));
    expect(calls()).toBe(1);
    update(() => AppEnv.config.set(SHOW_NAV_RAIL_CONFIG_KEY, true));
    expect(calls()).toBe(2);
  });

  it('leaves the toolbars alone when neither the rail nor the toolbar changes', () => {
    update(() => {});
    update(() => stack.push({ id: 'Thread' }));
    expect(calls()).toBe(0);
  });
});
