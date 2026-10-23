import React from 'react';
import ReactDOM from 'react-dom';
import path from 'path';

import { EventedIFrame } from '../../components/evented-iframe';
import Package from '../../packages/package';
import {
  themePreviewColors,
  themePreviewColorsCSS,
  ThemePreviewColors,
} from '../../packages/theme-preview';
import { RadioGroupItem } from '../../components/kit/radio-group';

// One tile in the theme picker's RadioGroup; its value is the theme name.
interface ThemeOptionProps {
  theme: Package;
  active: boolean;
}

export function toSelector(themeName: string) {
  return `theme-preview-${btoa(themeName).replace(/=/g, 'z')}`;
}

class ThemeOption extends React.Component<ThemeOptionProps> {
  _iframeComponent: EventedIFrame | null = null;

  componentDidMount() {
    this._writeContent();
  }

  // The iframe has no tokens: the tile gets the theme's preview colours as --theme-preview-*
  // properties (src/packages/theme-preview.ts) ahead of its stylesheet.
  _previewCSS() {
    let colors: ThemePreviewColors = {};
    try {
      colors = themePreviewColors(this.props.theme);
    } catch (err) {
      const { name } = this.props.theme;
      console.warn(`Could not preview the theme ${name}: ${(err as Error).message}`);
    }
    const { resourcePath } = AppEnv.getLoadSettings();
    const sheet = AppEnv.themes.cssContentsOfStylesheet(
      path.join(resourcePath, 'src/features/theme-picker/preview-styles/theme-option.css')
    );
    return `${themePreviewColorsCSS(colors)}\n${sheet}`;
  }

  _writeContent() {
    const doc = (ReactDOM.findDOMNode(this._iframeComponent) as HTMLIFrameElement).contentDocument;
    if (!doc) return;

    const css = `<style>${this._previewCSS()}</style>`;
    const html = `<!DOCTYPE html>
                  ${css}
                  <body>
                    <div class="theme-option active-${this.props.active}">
                      <div class="theme-name ">${this.props.theme.displayName}</div>
                      <div class="swatches" style="display:flex;flex-direction:row;">
                        <div class="swatch font-color"></div>
                        <div class="swatch active-color"></div>
                        <div class="swatch toolbar-color"></div>
                      </div>
                      <div class="divider-black"></div>
                      <div class="divider-white"></div>
                      <div class="strip"></div>
                    </div>
                  </body>`;

    doc.open();
    doc.write(html);
    doc.close();
  }

  render() {
    return (
      <RadioGroupItem
        value={this.props.theme.name}
        aria-label={this.props.theme.displayName}
        className="clickable-theme-option"
      >
        <EventedIFrame
          ref={(cm: EventedIFrame | null) => {
            this._iframeComponent = cm;
          }}
          className={toSelector(this.props.theme.name)}
          frameBorder="0"
          width="115px"
          height="70px"
        />
      </RadioGroupItem>
    );
  }
}

export default ThemeOption;
