import fs from 'fs';
import os from 'os';
import path from 'path';
import vm from 'vm';
import { bundleScript } from '../../src/main/frame/script-bundler';
import {
  messageFramePage,
  messageFrameResponse,
  messageFrameScript,
} from '../../src/main/frame/protocol-handler';
import { messageFrameContentSecurityPolicy } from '../../src/shared/content-security-policy';
import { startMessageFrame, normalizeHref } from '../../src/mail/frame/runtime';
import { MessageBody, MessageFrameBridge, messageFrameContent } from '../../src/mail/frame/bridge';
import {
  FrameToAppMessage,
  RenderMessage,
  messageFrameURL,
} from '../../src/shared/message-frame-protocol';
import { shouldBlockFrameNavigation } from '../../src/main/frame/navigation';

const resourcePath = () => AppEnv.getLoadSettings().resourcePath;

const realDelay = (ms: number) =>
  new Promise((resolve) => (window as any).originalSetTimeout(resolve, ms));

async function waitFor<T>(fn: () => T | null | undefined | false, what: string): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const value = fn();
    if (value) return value;
    await realDelay(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

function render(html: string, extra: Partial<RenderMessage> = {}): RenderMessage {
  return {
    type: 'render',
    generation: 1,
    html,
    plaintext: false,
    themeStyles: '',
    platform: process.platform,
    restrictWidth: false,
    colorScheme: 'light',
    darkPalette: null,
    themeBackground: { r: 255, g: 255, b: 255, a: 1 },
    ...extra,
  };
}

describe('message frame', function () {
  describe('page and script', () => {
    it('serves the page with the frame policy as a header and as its first element', () => {
      for (const remoteImages of [false, true]) {
        const url = messageFrameURL({ remoteImages });
        const { policy, html } = messageFramePage(url);
        expect(policy).toEqual(messageFrameContentSecurityPolicy({ remoteImages }));
        expect(html.indexOf('Content-Security-Policy')).toBeLessThan(html.indexOf('<script'));
        const response = messageFrameResponse(url, 'GET', () => '');
        expect(response.headers.get('Content-Security-Policy')).toEqual(policy);
      }
      expect(messageFrameContentSecurityPolicy({ remoteImages: false })).not.toContain('https:');
      expect(messageFrameContentSecurityPolicy({ remoteImages: true })).toContain('https:');
      expect(messageFrameContentSecurityPolicy({ remoteImages: false })).toContain(
        'script-src tern-message://frame/runtime.js'
      );
    });

    it('refuses other paths, other query strings and other methods', () => {
      expect(messageFrameResponse('tern-message://frame/other.html', 'GET', () => '').status).toBe(
        404
      );
      expect(
        messageFrameResponse('tern-message://frame/message.html?remote-images=2', 'GET', () => '')
          .status
      ).toBe(404);
      expect(
        messageFrameResponse(messageFrameURL({ remoteImages: false }), 'POST', () => '').status
      ).toBe(405);
    });

    it('bundles the frame runtime without Node modules', () => {
      const script = messageFrameScript(resourcePath());
      expect(script).toContain('startMessageFrame');
      expect(script).not.toMatch(/require\(["'](fs|path|electron)["']\)/);
    });

    it('refuses to bundle a module that needs Node', () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'frame-bundle-'));
      try {
        fs.writeFileSync(path.join(dir, 'entry.js'), "require('./dep');");
        fs.writeFileSync(path.join(dir, 'dep.js'), "module.exports = require('fs');");
        expect(() => bundleScript(path.join(dir, 'entry.js'))).toThrow();
      } finally {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    });

    it('runs a bundle on its own require', () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'frame-bundle-'));
      try {
        fs.writeFileSync(
          path.join(dir, 'entry.ts'),
          "import { value } from './dep';\n(globalThis as any).__frameBundleSpec = value * 2;"
        );
        fs.writeFileSync(path.join(dir, 'dep.ts'), 'export const value: number = 21;');
        const context: any = {};
        vm.runInNewContext(bundleScript(path.join(dir, 'entry.ts')), context);
        expect(context.__frameBundleSpec).toBe(42);
      } finally {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    });
  });

  describe('navigation', () => {
    it('lets the app load the frame page and blocks the frame from navigating itself', () => {
      const page = messageFrameURL({ remoteImages: true });
      expect(
        shouldBlockFrameNavigation({
          url: page,
          isMainFrame: false,
          initiatorUrl: 'file:///app/static/index.html',
        })
      ).toBe(false);
      expect(
        shouldBlockFrameNavigation({
          url: page,
          isMainFrame: false,
          initiatorUrl: messageFrameURL({ remoteImages: false }),
        })
      ).toBe(true);
      expect(shouldBlockFrameNavigation({ url: page, isMainFrame: false })).toBe(true);
      expect(
        shouldBlockFrameNavigation({
          url: 'tern-message://frame/message.html?remote-images=1&x',
          isMainFrame: false,
          initiatorUrl: 'file:///app/static/index.html',
        })
      ).toBe(true);
    });
  });

  // The runtime in a same-origin frame, so the spec can click inside it; the real frame is
  // cross-origin (email-frame-csp-spec loads it).
  describe('runtime and bridge', () => {
    let iframe: HTMLIFrameElement;
    let bridge: MessageFrameBridge;
    let received: FrameToAppMessage[];

    const lastOf = (type: string) => [...received].reverse().find((m) => m.type === type) as any;

    beforeEach(async () => {
      received = [];
      iframe = document.createElement('iframe');
      iframe.style.width = '600px';
      document.body.appendChild(iframe);
      const doc = iframe.contentDocument;
      doc.open();
      doc.write('<!DOCTYPE html><html><head></head><body></body></html>');
      doc.close();
      bridge = new MessageFrameBridge(iframe, (msg) => received.push(msg));
      const frameWindow = iframe.contentWindow;
      startMessageFrame(frameWindow, (msg) =>
        window.dispatchEvent(new MessageEvent('message', { data: msg, source: frameWindow }))
      );
      await waitFor(() => bridge.ready, 'ready');
    });

    afterEach(() => {
      bridge.dispose();
      iframe.remove();
    });

    const renderAndWait = async (html: string, extra: Partial<RenderMessage> = {}) => {
      const generation = bridge.nextGeneration();
      bridge.post(render(html, { generation, ...extra }));
      await waitFor(
        () => received.find((m) => m.type === 'rendered' && m.generation === generation),
        'rendered'
      );
      return iframe.contentDocument;
    };

    it('renders the body, reports its height and returns its content without scripts', async () => {
      const doc = await renderAndWait('<p style="height: 300px">Hello</p>');
      expect(doc.getElementById('inbox-html-wrapper').textContent).toBe('Hello');
      const size = await waitFor(() => lastOf('size'), 'size');
      expect(size.height).toBeGreaterThan(299);

      doc.head.appendChild(doc.createElement('script'));
      const content = await messageFrameContent(iframe);
      expect(content.text).toContain('Hello');
      expect(content.html).toContain('inbox-html-wrapper');
      expect(content.html).not.toContain('<script');
    });

    it('renders plaintext as text', async () => {
      const doc = await renderAndWait('<b>not bold</b>', { plaintext: true });
      const wrapper = doc.getElementById('inbox-plain-wrapper');
      expect(wrapper.textContent).toBe('<b>not bold</b>');
      expect(wrapper.querySelector('b')).toBe(null);
    });

    it('sends link clicks, fixing hrefs without a scheme and dropping file: links', async () => {
      const doc = await renderAndWait(
        '<a id="a" href="https://example.com/x">a</a><a id="b" href="example.com">b</a>' +
          '<a id="c" href="//example.org">c</a><a id="d" href="file:///etc/passwd">d</a>'
      );
      for (const id of ['a', 'b', 'c', 'd']) {
        doc
          .getElementById(id)
          .dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
      }
      await realDelay(20);
      expect(received.filter((m) => m.type === 'link').map((m: any) => m.href)).toEqual([
        'https://example.com/x',
        'http://example.com',
        'https://example.org',
      ]);
      expect(normalizeHref('mailto:a@b.c')).toBe('mailto:a@b.c');
    });

    it('sends forms to the app instead of submitting them', async () => {
      const doc = await renderAndWait(
        '<form id="f" action="https://example.com/s" method="get"><input name="q" value="1"></form>'
      );
      const form = doc.getElementById('f') as HTMLFormElement;
      const event = new (iframe.contentWindow as any).SubmitEvent('submit', {
        bubbles: true,
        cancelable: true,
      });
      form.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
      const msg = await waitFor(() => lastOf('form'), 'form');
      expect(msg).toEqual({ type: 'form', url: 'https://example.com/s?q=1', method: 'get' });
    });

    it('forwards keys without modifiers and the context menu target', async () => {
      const doc = await renderAndWait(
        '<a id="a" href="https://example.com"><img src="data:,x">x</a>'
      );
      const KeyboardEventInFrame = (iframe.contentWindow as any).KeyboardEvent;
      doc.dispatchEvent(new KeyboardEventInFrame('keydown', { key: 'j', bubbles: true }));
      doc.dispatchEvent(
        new KeyboardEventInFrame('keydown', { key: 'c', metaKey: true, bubbles: true })
      );
      doc
        .querySelector('img')
        .dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
      await realDelay(20);
      expect(received.filter((m) => m.type === 'key').map((m: any) => m.event.key)).toEqual(['j']);
      const menu = lastOf('contextmenu');
      expect(menu.href).toBe('https://example.com');
      expect(menu.src).toBe('data:,x');
    });

    it('highlights search matches and reports them in render order', async () => {
      const doc = await renderAndWait('<p>needle one</p><p>needle two</p>');
      bridge.post({ type: 'search', regionId: 'r1', term: 'needle', currentIndex: 1 });
      const results = await waitFor(() => lastOf('searchResults'), 'searchResults');
      expect(results.regionId).toBe('r1');
      expect(results.matches.map((m) => m.renderIndex)).toEqual([0, 1]);
      expect(doc.querySelectorAll('search-match').length).toBe(2);
      expect(doc.querySelector('search-match.current-match').textContent).toBe('needle');

      bridge.post({ type: 'search', regionId: 'r1', term: null, currentIndex: null });
      await waitFor(() => doc.querySelectorAll('search-match').length === 0, 'cleared');
    });

    it('lets extensions edit the body through MessageBody, within limits', async () => {
      await renderAndWait('<a href="https://t.example/1">one</a>');
      const body = new MessageBody(bridge, bridge.generation);
      const [anchor] = await body.querySelectorAll('a[href]');
      expect(anchor.attributes.href).toBe('https://t.example/1');
      await body.setAttributes(anchor.ref, { href: 'https://example.com/1', title: null });
      const dot = await body.insertElement(anchor.ref, 'afterend', 'img', {
        class: 'dot',
        src: 'data:,x',
      });
      const doc = iframe.contentDocument;
      expect(doc.querySelector('a').getAttribute('href')).toBe('https://example.com/1');
      expect(doc.querySelector('a').nextElementSibling.className).toBe('dot');

      let refused = 0;
      await body.setAttributes(anchor.ref, { onclick: 'x()' }).catch(() => refused++);
      await body.insertElement(anchor.ref, 'afterend', 'script').catch(() => refused++);
      await body
        .insertElement(anchor.ref, 'afterend', 'img', { onerror: 'x()' })
        .catch(() => refused++);
      expect(refused).toBe(3);

      const clicks = [];
      await body.addEventListener(dot, 'mousedown', (e) => clicks.push(e.rect));
      doc.querySelector('.dot').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
      await waitFor(() => clicks.length > 0, 'listener');
      const frameRect = iframe.getBoundingClientRect();
      expect(clicks[0].top).toBeGreaterThan(frameRect.top - 1);

      // A new render makes the old refs and listeners stale.
      await renderAndWait('<p>again</p>');
      let stale = null;
      await body.querySelectorAll('p').catch((e) => (stale = e));
      expect(stale).not.toBe(null);
    });
  });
});
