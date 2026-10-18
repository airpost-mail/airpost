/**
 * Whether a navigation in a Tern window should be cancelled. The window itself never
 * navigates, and message frames are written in place into about:blank, so a subframe
 * navigating anywhere else comes from message content: a form, a meta refresh or a link
 * EventedIFrame didn't catch.
 */
export function shouldBlockFrameNavigation({
  url,
  isMainFrame,
}: {
  url: string;
  isMainFrame: boolean;
}) {
  if (isMainFrame) return true;
  return url !== 'about:blank' && url !== 'about:srcdoc';
}
