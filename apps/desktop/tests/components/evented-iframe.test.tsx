import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';
import fs from 'fs';
import path from 'path';
import { ipcRenderer } from 'electron';
const { EventedIFrame, copyImageToClipboard } = require('../../src/components/evented-iframe');

const PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNikAQAACIAHF/uBd8AAAAASUVORK5CYII=';

describe('EventedIFrame', () =>
  describe('link clicking behavior', function () {
    beforeEach(function () {
      this.frame = ReactTestUtils.renderIntoDocument(<EventedIFrame src="about:blank" />);

      this.setAttributeSpy = jasmine.createSpy('setAttribute');
      this.preventDefaultSpy = jasmine.createSpy('preventDefault');
      this.openLinkSpy = jasmine.createSpy('openLink');

      this.oldOpenLink = AppEnv.windowEventHandler.openLink;
      AppEnv.windowEventHandler.openLink = this.openLinkSpy;

      this.fakeEvent = (href) => {
        return {
          stopPropagation() {},
          preventDefault: this.preventDefaultSpy,
          target: {
            getAttribute(attr) {
              return href;
            },
            setAttribute: this.setAttributeSpy,
          },
        };
      };
    });

    afterEach(function () {
      AppEnv.windowEventHandler.openLink = this.oldOpenLink;
    });

    it('works for acceptable link types', function () {
      const hrefs = [
        'http://example.com',
        'https://www.example.com',
        'mailto:evan@example.com',
        'tel:8585311718',
        'custom:www.example.com',
      ];
      for (let i = 0; i < hrefs.length; i++) {
        const href = hrefs[i];
        this.frame._onIFrameClick(this.fakeEvent(href));
        expect(this.setAttributeSpy).not.toHaveBeenCalled();
        expect(this.openLinkSpy).toHaveBeenCalled();
        const { target } = this.openLinkSpy.calls[i].args[0];
        const targetHref = this.openLinkSpy.calls[i].args[0].href;
        expect(target).not.toBeDefined();
        expect(targetHref).toBe(href);
      }
    });

    it('corrects relative uris', function () {
      const hrefs = ['example.com', 'www.example.com'];
      for (let i = 0; i < hrefs.length; i++) {
        const href = hrefs[i];
        this.frame._onIFrameClick(this.fakeEvent(href));
        expect(this.setAttributeSpy).toHaveBeenCalled();
        const modifiedHref = this.setAttributeSpy.calls[i].args[1];
        expect(modifiedHref).toBe(`http://${href}`);
      }
    });

    it('corrects protocol-relative uris', function () {
      const hrefs = ['//example.com', '//www.example.com'];
      for (let i = 0; i < hrefs.length; i++) {
        const href = hrefs[i];
        this.frame._onIFrameClick(this.fakeEvent(href));
        expect(this.setAttributeSpy).toHaveBeenCalled();
        const modifiedHref = this.setAttributeSpy.calls[i].args[1];
        expect(modifiedHref).toBe(`https:${href}`);
      }
    });

    it('disallows malicious uris', function () {
      const hrefs = ['file://usr/bin/bad'];
      for (const href of Array.from(hrefs)) {
        this.frame._onIFrameClick(this.fakeEvent(href));
        expect(this.preventDefaultSpy).toHaveBeenCalled();
        expect(this.openLinkSpy).not.toHaveBeenCalled();
      }
    });
  }));

describe('EventedIFrame form submission', function () {
  let container: HTMLDivElement;
  let frame: any;
  let doc: Document;
  let openLinkSpy: jasmine.Spy;
  let confirmSpy: jasmine.Spy;
  let oldOpenLink;

  const writeForm = (html: string) => {
    doc = (ReactDOM.findDOMNode(frame) as HTMLIFrameElement).contentDocument;
    doc.open();
    doc.write(`<!DOCTYPE html><body>${html}</body>`);
    doc.close();
    frame.didReplaceDocument();
    advanceClock(); // EventedIFrame attaches its listeners in a _.defer
  };

  beforeEach(function () {
    container = document.createElement('div');
    document.body.appendChild(container);
    frame = ReactDOM.render(<EventedIFrame sandbox="allow-forms allow-same-origin" />, container);
    oldOpenLink = AppEnv.windowEventHandler.openLink;
    openLinkSpy = jasmine.createSpy('openLink');
    AppEnv.windowEventHandler.openLink = openLinkSpy;
    confirmSpy = spyOn(frame, '_confirmFormSubmission').andReturn(true);
  });

  afterEach(function () {
    AppEnv.windowEventHandler.openLink = oldOpenLink;
    ReactDOM.unmountComponentAtNode(container);
    container.remove();
  });

  it('opens a GET form in the browser with its fields after confirming, never in the frame', async () => {
    writeForm(`
      <form id="f" action="https://rsvp.example.com/answer" method="get">
        <input name="name" value="Ann Lee">
        <input type="radio" name="answer" value="yes" checked>
        <input type="radio" name="answer" value="no">
        <button id="go" type="submit">Send</button>
      </form>`);
    (doc.getElementById('f') as HTMLFormElement).requestSubmit(doc.getElementById('go'));

    expect(confirmSpy).toHaveBeenCalled();
    expect(confirmSpy.calls[0].args[0]).toEqual({
      url: 'https://rsvp.example.com/answer?name=Ann+Lee&answer=yes',
      method: 'get',
    });
    expect(openLinkSpy).toHaveBeenCalledWith({
      href: 'https://rsvp.example.com/answer?name=Ann+Lee&answer=yes',
    });

    await new Promise((resolve) => (window as any).originalSetTimeout(resolve, 200));
    const current = (ReactDOM.findDOMNode(frame) as HTMLIFrameElement).contentDocument;
    expect(current.getElementById('f')).not.toBe(null);
  });

  it('opens a POST form without its fields, honouring formaction on the button', () => {
    writeForm(`
      <form id="f" action="https://login.example.com/form" method="post">
        <input name="password" value="hunter2">
        <button id="go" formaction="https://phish.example.net/collect">Sign in</button>
      </form>`);
    (doc.getElementById('f') as HTMLFormElement).requestSubmit(doc.getElementById('go'));

    expect(confirmSpy.calls[0].args[0]).toEqual({
      url: 'https://phish.example.net/collect',
      method: 'post',
    });
    expect(openLinkSpy).toHaveBeenCalledWith({ href: 'https://phish.example.net/collect' });
  });

  it('opens nothing when the user declines', () => {
    confirmSpy.andReturn(false);
    writeForm(`<form id="f" action="https://example.com/x"><input name="q" value="1"></form>`);
    (doc.getElementById('f') as HTMLFormElement).requestSubmit();
    expect(confirmSpy).toHaveBeenCalled();
    expect(openLinkSpy).not.toHaveBeenCalled();
  });

  it('drops forms whose action is not an absolute http(s) URL', () => {
    writeForm(`
      <form id="relative" action="/submit"><input name="q" value="1"></form>
      <form id="none"><input name="q" value="1"></form>
      <form id="file" action="file:///etc/passwd"><input name="q" value="1"></form>`);
    for (const id of ['relative', 'none', 'file']) {
      (doc.getElementById(id) as HTMLFormElement).requestSubmit();
    }
    expect(confirmSpy).not.toHaveBeenCalled();
    expect(openLinkSpy).not.toHaveBeenCalled();
  });
});

describe('EventedIFrame Copy Image', function () {
  const dir = () => path.join(AppEnv.getConfigDirPath(), 'files', 'zz', 'zz', 'copy-image-spec');

  beforeEach(() => {
    fs.mkdirSync(dir(), { recursive: true });
    fs.writeFileSync(path.join(dir(), 'logo.png'), Buffer.from(PNG_BASE64, 'base64'));
    spyOn(ipcRenderer, 'send');
  });

  afterEach(() => {
    fs.rmSync(dir(), { recursive: true, force: true });
  });

  // A tern-file: image is cross-origin to the app document and taints a canvas it is drawn on.
  for (const [label, src] of [
    ['an inline tern-file: image', 'tern-file:///zz/zz/copy-image-spec/logo.png'],
    ['a data: image', `data:image/png;base64,${PNG_BASE64}`],
  ]) {
    it(`copies ${label} as PNG`, async () => {
      await copyImageToClipboard(src);
      expect(ipcRenderer.send).toHaveBeenCalled();
      const [channel, dataURL] = (ipcRenderer.send as jasmine.Spy).calls[0].args;
      expect(channel).toEqual('write-image-to-clipboard');
      expect(dataURL).toMatch(/^data:image\/png;base64,/);
    });
  }

  it('copies nothing for a tern-file: URL outside the files directory', async () => {
    let error = null;
    try {
      await copyImageToClipboard('tern-file:///zz/zz/copy-image-spec/../../../../secret.png');
    } catch (err) {
      error = err;
    }
    expect(error).not.toBe(null);
    expect(ipcRenderer.send).not.toHaveBeenCalled();
  });
});
