import { shouldBlockFrameNavigation } from '../../src/main/frame-navigation';

describe('shouldBlockFrameNavigation', function () {
  it('cancels every top-frame navigation', () => {
    expect(shouldBlockFrameNavigation({ url: 'https://example.com', isMainFrame: true })).toBe(
      true
    );
    expect(shouldBlockFrameNavigation({ url: 'about:blank', isMainFrame: true })).toBe(true);
  });

  it('lets subframes stay on about:blank and blocks them from going anywhere else', () => {
    expect(shouldBlockFrameNavigation({ url: 'about:blank', isMainFrame: false })).toBe(false);
    expect(shouldBlockFrameNavigation({ url: 'about:srcdoc', isMainFrame: false })).toBe(false);
    for (const url of [
      'https://phish.example.net/collect',
      'http://example.com/?q=1',
      'file:///etc/passwd',
      'data:text/html,<b>hi</b>',
      'about:blank#x',
    ]) {
      expect([url, shouldBlockFrameNavigation({ url, isMainFrame: false })]).toEqual([url, true]);
    }
  });
});
