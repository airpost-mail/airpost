import { useEffect, useMemo, useRef, useState } from 'react';
import { cn } from '@/lib/utils';
import { buildSrcDoc, prepareHtml } from './sanitize';

export interface EmailFrameProps {
  html: string;
  allowRemoteImages?: boolean;
  /** Maps a `cid:` reference to a URL for the inline part, or null to leave it broken. */
  resolveCid?: (cid: string) => string | null;
  /** Links are opened by the app, never by the frame. Defaults to a new tab, noopener. */
  onLinkClick?: (url: string) => void;
  className?: string;
  title?: string;
}

function openExternal(url: string) {
  window.open(url, '_blank', 'noopener,noreferrer');
}

const XLINK = 'http://www.w3.org/1999/xlink';

/** The href of the link-like element around `target`: HTML a and area, and SVG a. */
function linkAt(target: EventTarget | null): { el: Element; href: string } | null {
  const el = (target as Element | null)?.closest?.('a, area');
  if (!el) return null;
  const href =
    el.getAttribute('href') ??
    el.getAttributeNS(XLINK, 'href') ??
    el.getAttribute('xlink:href') ??
    '';
  return { el, href: href.trim() };
}

/**
 * Renders an HTML email in a sandboxed iframe: no scripts, no forms, no navigation,
 * remote images blocked by CSP unless allowed. `allow-same-origin` (without
 * `allow-scripts`) only lets this component measure the height and catch link clicks.
 */
export function EmailFrame({
  html,
  allowRemoteImages = false,
  resolveCid,
  onLinkClick = openExternal,
  className,
  title = 'Message body',
}: EmailFrameProps) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(120);
  const linkRef = useRef(onLinkClick);
  useEffect(() => {
    linkRef.current = onLinkClick;
  });

  const srcDoc = useMemo(
    () => buildSrcDoc(prepareHtml(html, resolveCid), { allowRemoteImages }),
    [html, resolveCid, allowRemoteImages],
  );

  useEffect(() => {
    const frame = ref.current;
    if (!frame) return;
    let observer: ResizeObserver | null = null;
    let restored = false;

    const onLoad = () => {
      const doc = frame.contentDocument;
      // Something navigated the frame away from our document (a cross-origin page leaves
      // contentDocument null). Put the message back rather than show that page.
      if (!doc || doc.URL !== 'about:srcdoc') {
        // Once only: if the frame can't show our document at all, don't loop.
        if (!restored) {
          restored = true;
          frame.srcdoc = srcDoc;
        }
        return;
      }
      restored = false;
      const measure = () => setHeight(Math.ceil(doc.documentElement.scrollHeight));
      measure();
      if (typeof ResizeObserver !== 'undefined') {
        observer?.disconnect();
        observer = new ResizeObserver(measure);
        observer.observe(doc.documentElement);
      }
      const onClick = (e: MouseEvent) => {
        const link = linkAt(e.target);
        if (!link) return;
        // Every link, followable or not, so the frame never navigates itself.
        e.preventDefault();
        if (e.type === 'click' && /^(https?:|mailto:)/i.test(link.href)) linkRef.current(link.href);
      };
      doc.addEventListener('click', onClick);
      doc.addEventListener('auxclick', onClick);
    };

    frame.addEventListener('load', onLoad);
    return () => {
      frame.removeEventListener('load', onLoad);
      observer?.disconnect();
    };
  }, [srcDoc]);

  return (
    <iframe
      ref={ref}
      title={title}
      sandbox="allow-same-origin"
      referrerPolicy="no-referrer"
      srcDoc={srcDoc}
      style={{ height }}
      className={cn('block w-full rounded-md border-0 bg-white', className)}
    />
  );
}
