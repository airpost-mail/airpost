import React from 'react';
import ReactDOM from 'react-dom';
import { Message } from 'mailspring-exports';
import EmailFrame from '../lib/email-frame';
import { messageContentSecurityPolicy } from '../../../src/content-security-policy';

const BODY = '<p>Hello</p><img src="https://tracker.example.com/open.gif">';

describe('EmailFrame Content-Security-Policy', function () {
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

  const policyOf = (iframe: HTMLIFrameElement) => {
    const first = iframe.contentDocument.head.firstElementChild as HTMLMetaElement;
    expect(first.tagName).toBe('META');
    expect(first.httpEquiv).toBe('Content-Security-Policy');
    return first.content;
  };

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    container.remove();
  });

  it('writes the blocking policy first when remote images are blocked', () => {
    const message = new Message({ id: 'csp-html', body: BODY, plaintext: false });
    const iframe = render(message, false);
    expect(policyOf(iframe)).toEqual(messageContentSecurityPolicy({ remoteImages: false }));
    expect(iframe.getAttribute('data-remote-images')).toBe('blocked');
  });

  it('writes the policy into plaintext messages too', () => {
    const message = new Message({ id: 'csp-plain', body: 'Hello', plaintext: true });
    expect(policyOf(render(message, false))).toEqual(
      messageContentSecurityPolicy({ remoteImages: false })
    );
  });

  it('re-renders in a new frame with the allowing policy when images are allowed', () => {
    const message = new Message({ id: 'csp-switch', body: BODY, plaintext: false });
    const blocked = render(message, false);
    const allowed = render(message, true);
    // A document keeps every policy it has seen, so the frame itself must be new.
    expect(allowed).not.toBe(blocked);
    expect(policyOf(allowed)).toEqual(messageContentSecurityPolicy({ remoteImages: true }));
    expect(allowed.getAttribute('data-remote-images')).toBe('allowed');
    expect(allowed.contentDocument.querySelector('img').getAttribute('src')).toBe(
      'https://tracker.example.com/open.gif'
    );
  });
});
