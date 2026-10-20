import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';

import Package from '../../src/packages/package';
import ThemePicker from '../../src/features/theme-picker/theme-picker';

const { resourcePath } = AppEnv.getLoadSettings();
const light = new Package(`${resourcePath}/themes/ui-light`);
const dark = new Package(`${resourcePath}/themes/ui-dark`);

describe('ThemePicker', function themePicker() {
  beforeEach(() => {
    spyOn(AppEnv.themes, 'getAvailableThemes').andReturn([light, dark]);
    spyOn(AppEnv.themes, 'getActiveThemeSetting').andReturn(light);
    this.component = ReactTestUtils.renderIntoDocument(<ThemePicker />);
  });

  it('changes the active theme when a theme is clicked', () => {
    spyOn(ThemePicker.prototype, '_setActiveTheme').andCallThrough();
    spyOn(ThemePicker.prototype, '_rewriteIFrame');
    const themeOption = ReactDOM.findDOMNode(
      ReactTestUtils.scryRenderedDOMComponentsWithClass(this.component, 'clickable-theme-option')[1]
    );
    expect((themeOption as HTMLElement).getAttribute('role')).toBe('radio');
    ReactTestUtils.Simulate.click(themeOption as any);
    expect(ThemePicker.prototype._setActiveTheme).toHaveBeenCalledWith(dark.name);
  });

  it('is a radio group with the active theme checked', () => {
    const node = ReactDOM.findDOMNode(this.component) as HTMLElement;
    const group = node.querySelector('[role="radiogroup"]');
    expect(group.getAttribute('aria-label')).toBe('Themes');
    const options = Array.from(group.querySelectorAll('[role="radio"]'));
    expect(options.map((o) => o.getAttribute('aria-label'))).toEqual([
      light.displayName,
      dark.displayName,
    ]);
    expect(options.map((o) => o.getAttribute('aria-checked'))).toEqual(['true', 'false']);
  });
});
