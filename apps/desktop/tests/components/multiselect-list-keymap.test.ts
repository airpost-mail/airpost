import { MultiselectList } from '../../src/components/multiselect/list';

describe('MultiselectList keymap handlers', function () {
  it('lets the caller override a default handler', function () {
    const onEnter = jasmine.createSpy('onEnter');
    const list = new MultiselectList({
      className: 'test',
      columns: [],
      itemPropsProvider: () => ({}),
      keymapHandlers: { 'core:focus-item': onEnter },
    } as any);
    const handlers = (list as any)._globalKeymapHandlers();
    expect(handlers['core:focus-item']).toBe(onEnter);
    expect(typeof handlers['core:next-item']).toBe('function');
  });
});
