import React from 'react';
import ReactDOM from 'react-dom';
import { Message } from 'mailspring-exports';
import EmailFrame from '../../../src/features/message-list/email-frame';
import { MessageFrameBridge } from '../../../src/mail/frame/bridge';
import { renderMessageDocument } from '../../../src/mail/frame/document';
import { RenderMessage } from '../../../src/shared/message-frame-protocol';
import {
  DEFAULT_DARK_PALETTE,
  EMAIL_RENDER_MODE_KEY,
  OriginalColors,
  darkPalette,
  darkeningApplies,
  emailRenderMode,
} from '../../../src/features/message-list/email-render-mode';

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

  // EmailFrame decides the colours and the frame applies them (mail/frame/document.ts); the spec
  // takes what EmailFrame posts and renders it into a same-origin frame it can inspect.
  describe('in the email frame', () => {
    let container: HTMLDivElement;
    let target: HTMLIFrameElement;
    let posted: RenderMessage[];
    const message = new Message({ id: 'email-render-mode-spec', body: BODY, plaintext: false });

    const lastRender = () => {
      const msg = posted[posted.length - 1];
      target.remove();
      target = document.createElement('iframe');
      container.appendChild(target);
      const doc = target.contentDocument;
      doc.open();
      doc.write('<!DOCTYPE html><html><head></head><body></body></html>');
      doc.close();
      renderMessageDocument(doc, msg);
      return doc;
    };

    const render = () => {
      ReactDOM.render(
        <EmailFrame content={BODY} showQuotedText={false} message={message} />,
        container
      );
      return lastRender();
    };

    beforeEach(() => {
      posted = [];
      spyOn(MessageFrameBridge.prototype, 'post').andCallFake((msg: any) => {
        if (msg.type === 'render') posted.push(msg);
      });
      container = document.createElement('div');
      container.style.background = '#212121';
      container.style.width = '700px';
      document.body.appendChild(container);
      target = document.createElement('iframe');
    });

    afterEach(() => {
      ReactDOM.unmountComponentAtNode(container);
      container.remove();
      OriginalColors.setShowingOriginal([message.id], false);
    });

    it('darkens the email under a dark theme and shows the original on request', () => {
      const doc = render();
      expect(posted[posted.length - 1].colorScheme).toBe('dark');
      expect(doc.querySelector('style[data-email-dark-sheet]')).not.toBe(null);
      const bg = doc.defaultView.getComputedStyle(doc.getElementById('outer')).backgroundColor;
      expect(bg).toBe('rgb(33, 33, 33)');

      OriginalColors.setShowingOriginal([message.id], true);
      const original = lastRender();
      expect(posted[posted.length - 1].darkPalette).toBe(null);
      expect(original.querySelector('style[data-email-dark-sheet]')).toBe(null);
      expect(
        original.getElementById('inbox-html-wrapper').classList.contains('has-background')
      ).toBe(true);
    });

    it('keeps the original colours in original mode', () => {
      mode = 'original';
      const doc = render();
      expect(posted[posted.length - 1].colorScheme).toBe('light');
      expect(doc.querySelector('style[data-email-dark-sheet]')).toBe(null);
    });
  });
});
