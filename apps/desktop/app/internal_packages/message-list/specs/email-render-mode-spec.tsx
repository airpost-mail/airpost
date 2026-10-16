import React from 'react';
import ReactDOM from 'react-dom';
import { Message } from 'mailspring-exports';
import EmailFrame from '../lib/email-frame';
import {
  DEFAULT_DARK_PALETTE,
  EMAIL_RENDER_MODE_KEY,
  OriginalColors,
  darkPalette,
  darkeningApplies,
  emailRenderMode,
} from '../lib/email-render-mode';

const BODY = `
<table id="outer" width="100%" bgcolor="#ffffff"><tr>
  <td id="cell" style="color: #333333">Your order has shipped.</td>
</tr></table>`;

describe('email render mode', function () {
  let mode: unknown;

  beforeEach(() => {
    mode = undefined;
    const get = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === EMAIL_RENDER_MODE_KEY ? mode : get(key)
    );
  });

  it('reads the setting, treating the old light mode as original colours', () => {
    expect(emailRenderMode()).toBe('auto');
    mode = 'light';
    expect(emailRenderMode()).toBe('original');
    mode = 'dark';
    expect(emailRenderMode()).toBe('dark');
    mode = 'original';
    expect(emailRenderMode()).toBe('original');
  });

  it('darkens under dark themes in auto mode, always in dark mode, never in original', () => {
    const dark = { r: 33, g: 33, b: 33, a: 1 };
    const light = { r: 255, g: 255, b: 255, a: 1 };
    expect(darkeningApplies(dark)).toBe(true);
    expect(darkeningApplies(light)).toBe(false);
    mode = 'dark';
    expect(darkeningApplies(light)).toBe(true);
    mode = 'original';
    expect(darkeningApplies(dark)).toBe(false);
  });

  it('uses the theme colours as the palette when they are legible', () => {
    const dark = { r: 33, g: 33, b: 33, a: 1 };
    expect(darkPalette(dark, 'rgb(238, 238, 238)', 'rgb(90, 163, 255)')).toEqual({
      background: 'rgb(33, 33, 33)',
      text: 'rgb(238, 238, 238)',
      link: 'rgb(90, 163, 255)',
    });
    expect(darkPalette(dark, 'rgb(17, 17, 17)', 'rgb(0, 0, 238)')).toEqual({
      background: 'rgb(33, 33, 33)',
      text: DEFAULT_DARK_PALETTE.text,
      link: DEFAULT_DARK_PALETTE.link,
    });
    expect(darkPalette({ r: 255, g: 255, b: 255, a: 1 }, 'x', 'y')).toEqual(DEFAULT_DARK_PALETTE);
  });

  describe('in the email frame', () => {
    let container: HTMLDivElement;
    const message = new Message({ id: 'email-render-mode-spec', body: BODY, plaintext: false });

    const render = () => {
      ReactDOM.render(
        <EmailFrame content={BODY} showQuotedText={false} message={message} />,
        container
      );
      return container.querySelector('iframe').contentDocument;
    };

    beforeEach(() => {
      container = document.createElement('div');
      container.style.background = '#212121';
      container.style.width = '700px';
      document.body.appendChild(container);
    });

    afterEach(() => {
      ReactDOM.unmountComponentAtNode(container);
      container.remove();
      OriginalColors.setShowingOriginal([message.id], false);
    });

    it('darkens the email under a dark theme and shows the original on request', () => {
      const doc = render();
      expect(doc.querySelector('style[data-email-dark-sheet]')).not.toBe(null);
      const bg = doc.defaultView.getComputedStyle(doc.getElementById('outer')).backgroundColor;
      expect(bg).toBe('rgb(33, 33, 33)');

      OriginalColors.setShowingOriginal([message.id], true);
      const original = container.querySelector('iframe').contentDocument;
      expect(original.querySelector('style[data-email-dark-sheet]')).toBe(null);
      expect(
        original.getElementById('inbox-html-wrapper').classList.contains('has-background')
      ).toBe(true);
    });

    it('keeps the original colours in original mode', () => {
      mode = 'original';
      const doc = render();
      expect(doc.querySelector('style[data-email-dark-sheet]')).toBe(null);
    });
  });
});
