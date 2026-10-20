import fs from 'fs';
import path from 'path';
import {
  AppContentSecurityPolicy,
  contentSecurityPolicyMetaTag,
  messageContentSecurityPolicy,
  printContentSecurityPolicy,
  quickPreviewContentSecurityPolicy,
} from '../src/shared/content-security-policy';

const resourcePath = () => AppEnv.getLoadSettings().resourcePath;

function directives(policy: string) {
  const result: { [name: string]: string[] } = {};
  for (const part of policy.split(';')) {
    const [name, ...sources] = part.trim().split(/\s+/);
    if (name) result[name] = sources;
  }
  return result;
}

// Writes `body` into a fresh iframe under `policy` and collects the CSP violations it causes
// until every <img> has settled.
async function violationsFor(policy: string, body: string) {
  const iframe = document.createElement('iframe');
  document.body.appendChild(iframe);
  const doc = iframe.contentDocument;
  const violations: { directive: string; blockedURI: string }[] = [];
  try {
    doc.open();
    doc.write(`<!DOCTYPE html>${contentSecurityPolicyMetaTag(policy)}${body}`);
    doc.close();
    doc.addEventListener('securitypolicyviolation', (e: SecurityPolicyViolationEvent) => {
      violations.push({ directive: e.effectiveDirective, blockedURI: e.blockedURI });
    });
    const images = Array.from(doc.images);
    await Promise.all(
      images.map(
        (img) =>
          new Promise<void>((resolve) => {
            if (img.complete) return resolve();
            img.addEventListener('load', () => resolve());
            img.addEventListener('error', () => resolve());
          })
      )
    );
    // Background images have no load event; give their fetches a moment to start.
    doc.body.getBoundingClientRect();
    await new Promise((resolve) => (window as any).originalSetTimeout(resolve, 100));
    return violations;
  } finally {
    iframe.remove();
  }
}

describe('Content-Security-Policy', function () {
  describe('the app policy', () => {
    it('is the same string in every static page and the response header', () => {
      const staticDir = path.join(resourcePath(), 'static');
      const pages = fs.readdirSync(staticDir).filter((name) => name.endsWith('.html'));
      expect(pages).toContain('index.html');
      for (const page of pages) {
        const html = fs.readFileSync(path.join(staticDir, page), 'utf8');
        const match = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]*)">/);
        if (page === 'index.html') expect(match).not.toBe(null);
        if (match) expect(`${page}: ${match[1]}`).toEqual(`${page}: ${AppContentSecurityPolicy}`);
      }

      // main.js sends the constant rather than a copy of it.
      const main = fs.readFileSync(path.join(resourcePath(), 'src', 'main', 'main.js'), 'utf8');
      expect(main).toContain("'Content-Security-Policy': [AppContentSecurityPolicy]");
      expect(main).not.toContain('default-src');
    });

    it('is the policy of the window the specs run in', () => {
      const meta = document.querySelector(
        'meta[http-equiv="Content-Security-Policy"]'
      ) as HTMLMetaElement;
      expect(meta.content).toEqual(AppContentSecurityPolicy);
    });

    it('allows no inline or eval script and pins connect-src and frame-src', () => {
      const d = directives(AppContentSecurityPolicy);
      expect(d['script-src']).toEqual(["'self'"]);
      expect(d['default-src']).not.toContain('*');
      // Only the sandboxed message frame (src/message-frame).
      expect(d['frame-src']).toEqual(['tern-message://frame']);
      expect(d['object-src']).toEqual(["'none'"]);
      for (const source of d['connect-src']) {
        expect(['*', 'https:', 'http:', 'wss:', 'ws:', 'data:', 'blob:']).not.toContain(source);
      }
      expect(d['connect-src']).toContain('https://www.googleapis.com');
      expect(d['connect-src']).toContain('https://login.microsoftonline.com');
    });

    it('blocks inline script in the window', () => {
      (window as any).__cspInlineRan = false;
      const script = document.createElement('script');
      script.textContent = 'window.__cspInlineRan = true;';
      document.head.appendChild(script);
      script.remove();
      expect((window as any).__cspInlineRan).toBe(false);
    });
  });

  describe('the message policy', () => {
    it('blocks every remote source unless remote images are allowed', () => {
      const blocked = directives(messageContentSecurityPolicy({ remoteImages: false }));
      expect(blocked['default-src']).toEqual(["'none'"]);
      expect(blocked['style-src']).toEqual(["'unsafe-inline'"]);
      expect(blocked['img-src']).toEqual(['data:', 'mailspring:', 'tern-file:']);
      expect(blocked['font-src']).toEqual(['data:', 'mailspring:']);
      expect(blocked['form-action']).toEqual(["'none'"]);

      const allowed = directives(messageContentSecurityPolicy({ remoteImages: true }));
      expect(allowed['img-src']).toEqual(['data:', 'mailspring:', 'tern-file:', 'https:', 'http:']);
      expect(allowed['font-src']).toEqual(['data:', 'mailspring:', 'https:', 'http:']);
      expect(allowed['default-src']).toEqual(["'none'"]);
    });

    it('never allows file: in message documents', () => {
      for (const remoteImages of [false, true]) {
        for (const policy of [
          messageContentSecurityPolicy({ remoteImages }),
          printContentSecurityPolicy({ remoteImages }),
        ]) {
          expect(policy.split(/[\s;]+/)).not.toContain('file:');
          expect(directives(policy)['img-src']).not.toContain("'self'");
        }
      }
    });

    it('lets quick preview load remote images only when allowed', () => {
      expect(
        directives(quickPreviewContentSecurityPolicy({ remoteImages: false }))['img-src']
      ).toEqual(["'self'", 'data:']);
      expect(
        directives(quickPreviewContentSecurityPolicy({ remoteImages: true }))['img-src']
      ).toEqual(["'self'", 'data:', 'https:', 'http:']);
    });

    // Chromium reports a blocked file: URL as just "file".
    const isFileURI = (uri: string) => uri === 'file' || uri.startsWith('file:');

    // Port 9 is one Chromium refuses to connect to, so allowed loads fail at once without
    // touching the network.
    const REMOTE = 'https://127.0.0.1:9';
    const BODY = `
      <style>@font-face { font-family: Remote; src: url(${REMOTE}/font.woff2); }</style>
      <p style="font-family: Remote">Text</p>
      <img src="${REMOTE}/pixel.png">
      <div style="background:#fff url(${REMOTE}/background.png); width: 10px; height: 10px"></div>
      <div style="background-image: url(file:///etc/hosts); width: 10px; height: 10px"></div>
      <img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNikAQAACIAHF/uBd8AAAAASUVORK5CYII=">`;

    it('blocks remote and file: images in a message document when images are blocked', async () => {
      const violations = await violationsFor(
        messageContentSecurityPolicy({ remoteImages: false }),
        BODY
      );
      const blocked = violations.filter((v) => v.directive === 'img-src').map((v) => v.blockedURI);
      expect(blocked).toContain(`${REMOTE}/pixel.png`);
      expect(blocked).toContain(`${REMOTE}/background.png`);
      expect(blocked.some(isFileURI)).toBe(true);
      expect(blocked.some((uri) => uri.startsWith('data:'))).toBe(false);
      const fonts = violations.filter((v) => v.directive === 'font-src').map((v) => v.blockedURI);
      expect(fonts).toContain(`${REMOTE}/font.woff2`);
    });

    it('loads remote images but still blocks file: when images are allowed', async () => {
      const violations = await violationsFor(
        messageContentSecurityPolicy({ remoteImages: true }),
        BODY
      );
      const blocked = violations.filter((v) => v.directive === 'img-src').map((v) => v.blockedURI);
      expect(blocked.some((uri) => uri.startsWith(REMOTE))).toBe(false);
      expect(blocked.some(isFileURI)).toBe(true);
      expect(violations.some((v) => v.directive === 'font-src')).toBe(false);
    });
  });
});
