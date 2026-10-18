import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';
const EventedIFrame = require('../../src/components/evented-iframe').EventedIFrame;

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
        'http://mailspring.com',
        'https://www.mailspring.com',
        'mailto:evan@mailspring.com',
        'tel:8585311718',
        'custom:www.mailspring.com',
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
      const hrefs = ['mailspring.com', 'www.mailspring.com'];
      for (let i = 0; i < hrefs.length; i++) {
        const href = hrefs[i];
        this.frame._onIFrameClick(this.fakeEvent(href));
        expect(this.setAttributeSpy).toHaveBeenCalled();
        const modifiedHref = this.setAttributeSpy.calls[i].args[1];
        expect(modifiedHref).toBe(`http://${href}`);
      }
    });

    it('corrects protocol-relative uris', function () {
      const hrefs = ['//mailspring.com', '//www.mailspring.com'];
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
      for (let href of Array.from(hrefs)) {
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
