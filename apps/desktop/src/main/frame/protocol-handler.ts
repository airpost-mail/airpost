import { protocol } from 'electron';
import path from 'path';
import { bundleScript } from './script-bundler';
import {
  contentSecurityPolicyMetaTag,
  messageFrameContentSecurityPolicy,
} from '../../shared/content-security-policy';
import {
  MessageFrameOrigin,
  MessageFrameScheme,
  MessageFrameScriptPath,
  MessageFrameScriptURL,
  isMessageFrameURL,
  remoteImagesFromFrameURL,
} from '../../shared/message-frame-protocol';

/** The frame page for `url`, with its policy both as a header and as the first element. */
export function messageFramePage(url: string) {
  const policy = messageFrameContentSecurityPolicy({
    remoteImages: remoteImagesFromFrameURL(url),
  });
  const html =
    `<!DOCTYPE html><html><head>${contentSecurityPolicyMetaTag(policy)}` +
    `<meta charset="utf-8"><script defer src="${MessageFrameScriptURL}"></script>` +
    `</head><body></body></html>`;
  return { policy, html };
}

export function messageFrameResponse(url: string, method: string, script: () => string) {
  if (method !== 'GET') {
    return new Response('Method Not Allowed', { status: 405 });
  }
  if (isMessageFrameURL(url)) {
    const { policy, html } = messageFramePage(url);
    return new Response(html, {
      headers: {
        'Content-Type': 'text/html; charset=utf-8',
        'Content-Security-Policy': policy,
        'Cache-Control': 'no-store',
      },
    });
  }
  if (url === `${MessageFrameOrigin}${MessageFrameScriptPath}`) {
    return new Response(script(), {
      headers: { 'Content-Type': 'text/javascript; charset=utf-8' },
    });
  }
  return new Response('Not Found', { status: 404 });
}

export function messageFrameScript(resourcePath: string) {
  const dir = path.join(resourcePath, 'src', 'mail', 'frame');
  return bundleScript(require.resolve(path.join(dir, 'entry')), {
    'mailspring-exports': require.resolve(path.join(dir, 'exports')),
    // RegExpUtils.emojiRegex only; the frame never calls it.
    'emoji-data': null,
  });
}

// Serves the message frame (src/mail/frame). The script is bundled on first use.
export function registerMessageFrameProtocol(resourcePath: string) {
  let script: string | null = null;
  const getScript = () => {
    if (script === null) script = messageFrameScript(resourcePath);
    return script;
  };
  protocol.handle(MessageFrameScheme, (request) => {
    try {
      return messageFrameResponse(request.url, request.method, getScript);
    } catch (err) {
      console.error(`Could not serve ${request.url}: ${err}`);
      return new Response('Internal Error', { status: 500 });
    }
  });
}
