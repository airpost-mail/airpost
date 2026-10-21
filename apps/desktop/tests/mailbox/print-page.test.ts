// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import path from 'path';
import { pathToFileURL } from 'url';
import openPrintWindow, { inlineRelativeStyleURLs } from '../../src/features/print/window';
import { PrintWindows } from '../../src/platform/app-api';

describe('The print page', function () {
  const staticDir = path.join(AppEnv.getLoadSettings().resourcePath, 'static');
  const png = (rel: string) =>
    `url("data:image/png;base64,${fs.readFileSync(path.join(staticDir, rel)).toString('base64')}")`;

  beforeEach(() => {
    spyOn(PrintWindows, 'open').andReturn(Promise.resolve());
  });

  it("sends the page and messages to main, with the style tags' images inlined", async () => {
    const css =
      '<style>.a { background: url(../static/images/attachments/transparency-background.png) }' +
      ".b { background: url('images/menu/checked@2x.png') }</style>";
    await openPrintWindow({
      subject: 'Hi',
      account: { name: 'Me', email: 'me@example.com' },
      participants: [],
      styleTags: css,
      htmlContent: '',
      printMessages: '[]',
      remoteImages: false,
    });
    const { html, messagesScript } = (PrintWindows.open as jasmine.Spy).mostRecentCall.args[0];
    expect(messagesScript).toBe('window.printMessages = []');
    expect(html).toContain('<script type="text/javascript" src="print.messages.js"></script>');
    expect(html).toContain(png('images/attachments/transparency-background.png'));
    expect(html).toContain(png('images/menu/checked@2x.png'));
    expect(html).not.toContain('../static/');
  });

  it('leaves absolute, data:, fragment and missing URLs alone', () => {
    const base = `${pathToFileURL(path.join(staticDir, 'index.html')).href}?loadSettings=x`;
    const css =
      '.a { src: url("tern-asset://custom-fonts/fonts/AlbertSans-latin.woff2") }' +
      '.b { background: url(data:image/png;base64,AAAA) }' +
      '.c { filter: url(#f) }' +
      '.d { background: url(images/nope.png) }' +
      '.e { background: url(../../package.json) }';
    expect(inlineRelativeStyleURLs(css, base)).toBe(css);
  });
});
