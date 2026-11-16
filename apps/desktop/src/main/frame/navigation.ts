import { isMessageFrameURL } from '../../mail/frame/protocol';

/**
 * Whether a navigation in an Airpost window should be cancelled. The window itself never
 * navigates. A subframe may stay on about:blank (frames written in place) or load the
 * message frame page when the app window asks for it; anything else, and any navigation a
 * message frame starts itself (a meta refresh to the page variant that allows remote
 * images, say), comes from message content.
 */
export function shouldBlockFrameNavigation({
  url,
  isMainFrame,
  initiatorUrl,
}: {
  url: string;
  isMainFrame: boolean;
  initiatorUrl?: string | null;
}) {
  if (isMainFrame) return true;
  if (url === 'about:blank' || url === 'about:srcdoc') return false;
  if (isMessageFrameURL(url)) {
    return !initiatorUrl || initiatorUrl.startsWith('airpost-message:');
  }
  return true;
}
