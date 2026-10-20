// Runs inside the message frame (bundled by main/frame/protocol-handler.ts). It
// has no Node and no access to the app window: it renders what the app posts and posts back
// sizes, clicks, keys and search results. Messages from the app are trusted; what it sends
// back is checked by the app as content from the email.
import {
  BodyElements,
  decorateMessageDocument,
  measureMessageDocument,
  renderMessageDocument,
  searchMessageDocument,
} from './document';
import { formSubmissionTarget } from './form-submission';
import {
  AppToFrameMessage,
  ForwardedKeyEvent,
  ForwardedMouseEvent,
  FrameToAppMessage,
} from '../../shared/message-frame-protocol';

const KeyAttrs = [
  'key',
  'code',
  'location',
  'ctrlKey',
  'shiftKey',
  'altKey',
  'metaKey',
  'repeat',
  'isComposing',
  'charCode',
  'keyCode',
  'which',
];

function closestWith(target: EventTarget | null, attribute: string): HTMLElement | null {
  let node = target as HTMLElement | null;
  while (node && node.nodeType !== Node.ELEMENT_NODE) node = node.parentElement;
  while (node) {
    if (node.getAttribute && node.getAttribute(attribute) != null) return node;
    node = node.parentElement;
  }
  return null;
}

// An href with no scheme opens as https (protocol-relative) or http, as the app did before
// the frame was cross-origin.
export function normalizeHref(raw: string) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(raw)) return raw;
  if (/^\/\//.test(raw)) return `https:${raw}`;
  return `http://${raw}`;
}

/**
 * `post` sends to the app window; specs running this in a same-origin frame pass their own,
 * since a postMessage made from the app window's realm has the app window as its source.
 */
export function startMessageFrame(
  win: Window,
  post: (msg: FrameToAppMessage) => void = (msg) => win.parent.postMessage(msg, '*')
) {
  const doc = win.document;
  const app = win.parent;
  const elements = new BodyElements(doc);
  let generation = 0;
  let lastHeight = -1;
  let lastSearch: { regionId: string; term: string | null; currentIndex: number | null } = null;

  const send = post;

  const reportSize = () => {
    const height = measureMessageDocument(doc);
    if (height !== lastHeight) {
      lastHeight = height;
      send({ type: 'size', height });
    }
  };
  const observer = new (win as any).ResizeObserver(() =>
    win.requestAnimationFrame(reportSize)
  ) as ResizeObserver;
  observer.observe(doc.documentElement);
  if (doc.body) observer.observe(doc.body);

  const runSearch = () => {
    if (!lastSearch) return;
    const { regionId, term, currentIndex } = lastSearch;
    const matches = searchMessageDocument(doc, regionId, term, currentIndex);
    send({ type: 'searchResults', regionId, term, matches });
  };

  win.addEventListener('message', (event: MessageEvent) => {
    if (event.source !== app) return;
    const msg = event.data as AppToFrameMessage;
    if (!msg || typeof msg !== 'object') return;

    if (msg.type === 'render') {
      generation = msg.generation;
      elements.reset();
      renderMessageDocument(doc, msg);
      decorateMessageDocument(doc, { async: true });
      lastHeight = -1;
      reportSize();
      send({ type: 'rendered', generation });
      if (lastSearch && lastSearch.term) runSearch();
    } else if (msg.type === 'search') {
      lastSearch = { regionId: msg.regionId, term: msg.term, currentIndex: msg.currentIndex };
      runSearch();
    } else if (msg.type === 'request') {
      if (msg.generation !== generation) {
        send({ type: 'response', id: msg.id, error: 'the message was rendered again' });
        return;
      }
      try {
        const result = elements.run(msg, (listenerId, rect) =>
          send({ type: 'event', generation, listenerId, rect })
        );
        send({ type: 'response', id: msg.id, result });
      } catch (e) {
        send({ type: 'response', id: msg.id, error: String((e && e.message) || e) });
      }
    }
  });

  doc.addEventListener('click', (e: MouseEvent) => {
    e.stopPropagation();
    const target = closestWith(e.target, 'href');
    if (!target) return;
    e.preventDefault();
    const raw = target.getAttribute('href');
    // file: links never open; the app checks the rest again.
    if (/^file:/i.test(raw)) return;
    const href = normalizeHref(raw);
    if (href !== raw) target.setAttribute('href', href);
    send({ type: 'link', href, metaKey: e.metaKey });
  });

  // Forms never submit here; the app confirms and opens them in the browser.
  doc.addEventListener(
    'submit',
    (e: SubmitEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const submission = formSubmissionTarget(e.target as HTMLFormElement, e.submitter);
      if (submission) send({ type: 'form', url: submission.url, method: submission.method });
    },
    true
  );

  const onKey = (e: KeyboardEvent) => {
    // Shortcuts with modifiers reach the app's menu natively.
    if (e.metaKey || e.altKey || e.ctrlKey) return;
    const event = { type: e.type } as ForwardedKeyEvent;
    for (const attr of KeyAttrs) event[attr] = e[attr];
    send({ type: 'key', event });
  };
  doc.addEventListener('keydown', onKey);
  doc.addEventListener('keypress', onKey);
  doc.addEventListener('keyup', onKey);

  // The frame captures the mouse over it, so a drag that ends here would never end in the app.
  const onMouse = (e: MouseEvent) => {
    const event: ForwardedMouseEvent = {
      type: e.type as ForwardedMouseEvent['type'],
      clientX: e.clientX,
      clientY: e.clientY,
      screenX: e.screenX,
      screenY: e.screenY,
      button: e.button,
      buttons: e.buttons,
      detail: e.detail,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      metaKey: e.metaKey,
    };
    send({ type: 'mouse', event });
  };
  doc.addEventListener('mousedown', onMouse);
  doc.addEventListener('mousemove', onMouse);
  doc.addEventListener('mouseup', onMouse);

  doc.addEventListener('contextmenu', (e: MouseEvent) => {
    e.preventDefault();
    const link = closestWith(e.target, 'href');
    const image = closestWith(e.target, 'src');
    const selection = doc.getSelection();
    let text = selection && selection.rangeCount > 0 ? selection.getRangeAt(0).toString() : '';
    if (!text) {
      const el = link || (e.target as HTMLElement);
      text = (el && el.innerText) || '';
    }
    send({
      type: 'contextmenu',
      href: link ? link.getAttribute('href') : null,
      src: image ? image.getAttribute('src') : null,
      text,
    });
  });

  win.addEventListener('focus', () => send({ type: 'focus' }));
  win.addEventListener('blur', () => {
    const selection = doc.getSelection();
    if (selection) selection.empty();
  });

  send({ type: 'ready' });
}
