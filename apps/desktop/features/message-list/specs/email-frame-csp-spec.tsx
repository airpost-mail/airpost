import React from 'react';
import ReactDOM from 'react-dom';
import { Message } from 'mailspring-exports';
import EmailFrame from '../lib/email-frame';
import { messageFrameContentSecurityPolicy } from '../../../src/shared/content-security-policy';
import { messageFrameContent } from '../../../src/message-frame/message-frame-bridge';
import { messageFrameURL } from '../../../src/shared/message-frame-protocol';

const BODY = '<p>Hello</p><img src="https://tracker.example.com/open.gif">';

const realDelay = (ms: number) =>
  new Promise((resolve) => (window as any).originalSetTimeout(resolve, ms));

// Loads the real message frame from the main process: cross-origin, in its own process.
describe('EmailFrame in the message frame', function () {
  let container: HTMLDivElement;

  const render = (message: Message, remoteImages: boolean) => {
    ReactDOM.render(
      <EmailFrame
        content={message.body}
        showQuotedText={false}
        message={message}
        remoteImages={remoteImages}
      />,
      container
    );
    return container.querySelector('iframe');
  };

  const contentOf = async (iframe: HTMLIFrameElement) => {
    for (let i = 0; i < 300; i++) {
      const content = await messageFrameContent(iframe);
      if (content && content.text.includes('Hello')) return content;
      await realDelay(20);
    }
    throw new Error('the message frame never rendered');
  };

  beforeEach(() => {
    container = document.createElement('div');
    container.style.width = '600px';
    document.body.appendChild(container);
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    container.remove();
  });

  it('renders in a sandboxed cross-origin frame with the blocking policy', async () => {
    const message = new Message({ id: 'csp-html', body: BODY, plaintext: false });
    const iframe = render(message, false);
    expect(iframe.getAttribute('src')).toBe(messageFrameURL({ remoteImages: false }));
    expect(iframe.getAttribute('sandbox')).toBe('allow-scripts allow-forms');
    expect(iframe.getAttribute('data-remote-images')).toBe('blocked');

    const content = await contentOf(iframe);
    // Cross-origin: the app window can't reach into it.
    expect(iframe.contentDocument).toBe(null);
    expect(content.html).toContain(
      `content="${messageFrameContentSecurityPolicy({ remoteImages: false })}"`
    );
    expect(content.html).toContain('https://tracker.example.com/open.gif');
    expect(content.html).not.toContain('<script');
    expect(parseInt(iframe.style.height, 10)).toBeGreaterThan(0);

    // In a renderer process of its own, sandboxed (ProcessMetric.sandboxed is macOS and
    // Windows only).
    // Test-only introspection of the frame tree and process metrics.
    // eslint-disable-next-line no-restricted-syntax
    const remote = require('@electron/remote');
    const webContents = remote.getCurrentWebContents();
    const frame = webContents.mainFrame.framesInSubtree.find((f) =>
      f.url.startsWith('tern-message://frame/')
    );
    expect(frame).toBeDefined();
    expect(frame.osProcessId).not.toBe(webContents.getOSProcessId());
    if (process.platform !== 'linux') {
      const metric = remote.app.getAppMetrics().find((m) => m.pid === frame.osProcessId);
      expect(metric && metric.sandboxed).toBe(true);
    }
  });

  it('renders plaintext messages in the frame too', async () => {
    const message = new Message({ id: 'csp-plain', body: 'Hello <b>there</b>', plaintext: true });
    const content = await contentOf(render(message, false));
    expect(content.text).toContain('Hello <b>there</b>');
  });

  it('re-renders in a new frame with the allowing policy when images are allowed', async () => {
    const message = new Message({ id: 'csp-switch', body: BODY, plaintext: false });
    const blocked = render(message, false);
    const allowed = render(message, true);
    // A document keeps the policy it loaded with, so the frame itself must be new.
    expect(allowed).not.toBe(blocked);
    expect(allowed.getAttribute('src')).toBe(messageFrameURL({ remoteImages: true }));
    expect(allowed.getAttribute('data-remote-images')).toBe('allowed');
    const content = await contentOf(allowed);
    expect(content.html).toContain(
      `content="${messageFrameContentSecurityPolicy({ remoteImages: true })}"`
    );
  });
});
