import {
  AppToFrameMessage,
  BodyOperation,
  FrameRect,
  FrameToAppMessage,
} from './message-frame-protocol';
import type { BodyElementInfo } from './frame-document';

const RequestTimeout = 10000;

type Pending = { resolve: (value: unknown) => void; reject: (err: Error) => void; timer: any };

const bridges = new WeakMap<HTMLIFrameElement, MessageFrameBridge>();

/**
 * The app side of one message frame. Messages to the frame wait until it says `ready`;
 * messages from it are accepted only from that frame's window, and their content is the
 * email's, so the handlers check what they act on.
 */
export class MessageFrameBridge {
  iframe: HTMLIFrameElement;
  ready = false;
  generation = 0;
  _queue: AppToFrameMessage[] = [];
  _pending = new Map<number, Pending>();
  _nextId = 1;
  _onMessage: (msg: FrameToAppMessage) => void;
  _listeners = new Map<number, (e: { rect: FrameRect }) => void>();
  _disposed = false;

  constructor(iframe: HTMLIFrameElement, onMessage: (msg: FrameToAppMessage) => void) {
    this.iframe = iframe;
    this._onMessage = onMessage;
    window.addEventListener('message', this._onWindowMessage);
    bridges.set(iframe, this);
  }

  dispose() {
    this._disposed = true;
    window.removeEventListener('message', this._onWindowMessage);
    bridges.delete(this.iframe);
    for (const pending of this._pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error('the message frame is gone'));
    }
    this._pending.clear();
    this._listeners.clear();
  }

  post(msg: AppToFrameMessage) {
    if (this._disposed) return;
    if (!this.ready || !this.iframe.contentWindow) {
      // Only the latest render and search matter once the frame loads.
      this._queue = this._queue.filter((m) => m.type === 'request' || m.type !== msg.type);
      this._queue.push(msg);
      return;
    }
    this.iframe.contentWindow.postMessage(msg, '*');
  }

  /** Starts a new render; operations from earlier renders are refused from now on. */
  nextGeneration() {
    this.generation += 1;
    this._listeners.clear();
    return this.generation;
  }

  request<T>(operation: BodyOperation, generation = this.generation): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      if (this._disposed) {
        reject(new Error('the message frame is gone'));
        return;
      }
      const id = this._nextId++;
      const timer = setTimeout(() => {
        this._pending.delete(id);
        reject(new Error(`the message frame did not answer ${operation.op}`));
      }, RequestTimeout);
      this._pending.set(id, { resolve, reject, timer });
      this.post({ type: 'request', id, generation, ...operation } as AppToFrameMessage);
    });
  }

  listen(generation: number, ref: number, event: 'click' | 'mousedown', cb) {
    const listenerId = this._nextId++;
    this._listeners.set(listenerId, cb);
    return this.request<null>({ op: 'listen', ref, event, listenerId }, generation);
  }

  /** A rect from the frame in app window coordinates. */
  toWindowRect(rect: FrameRect) {
    const frame = this.iframe.getBoundingClientRect();
    const top = num(rect && rect.top) + frame.top;
    const left = num(rect && rect.left) + frame.left;
    const width = num(rect && rect.width);
    const height = num(rect && rect.height);
    return { top, left, width, height, bottom: top + height, right: left + width };
  }

  _onWindowMessage = (event: MessageEvent) => {
    if (this._disposed || !this.iframe.contentWindow) return;
    if (event.source !== this.iframe.contentWindow) return;
    const msg = event.data as FrameToAppMessage;
    if (!msg || typeof msg !== 'object' || typeof msg.type !== 'string') return;

    if (msg.type === 'ready') {
      this.ready = true;
      const queue = this._queue;
      this._queue = [];
      for (const queued of queue) this.iframe.contentWindow.postMessage(queued, '*');
    } else if (msg.type === 'response') {
      const pending = this._pending.get(msg.id);
      if (!pending) return;
      this._pending.delete(msg.id);
      clearTimeout(pending.timer);
      if (msg.error) pending.reject(new Error(String(msg.error)));
      else pending.resolve(msg.result);
    } else if (msg.type === 'event') {
      if (msg.generation !== this.generation) return;
      const cb = this._listeners.get(msg.listenerId);
      if (cb) cb({ rect: this.toWindowRect(msg.rect) });
    }
    this._onMessage(msg);
  };
}

function num(value: unknown) {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

export function messageFrameBridgeFor(iframe: Element | null): MessageFrameBridge | null {
  return iframe instanceof HTMLIFrameElement ? bridges.get(iframe) || null : null;
}

/**
 * The printed HTML (sender's colours) and text of the message in `iframe`, or null if it
 * isn't a message frame or doesn't answer.
 */
export async function messageFrameContent(
  iframe: Element | null
): Promise<{ html: string; text: string } | null> {
  const bridge = messageFrameBridgeFor(iframe);
  if (!bridge) return null;
  try {
    const result = await bridge.request<{ html: string; text: string }>({ op: 'content' });
    return {
      html: typeof result?.html === 'string' ? result.html : '',
      text: typeof result?.text === 'string' ? result.text : '',
    };
  } catch (err) {
    console.warn(`Could not read the message frame: ${err}`);
    return null;
  }
}

export type MessageBodyElement = BodyElementInfo;

export interface MessageBodyRect {
  top: number;
  left: number;
  bottom: number;
  right: number;
  width: number;
  height: number;
}

/**
 * The rendered message body as `MessageViewExtension.renderedMessageBodyIntoDocument` sees
 * it. The body is in a cross-origin sandboxed frame, so extensions work on it through these
 * calls instead of the DOM. Element refs are valid until the message renders again; calls
 * after that reject.
 */
export class MessageBody {
  _bridge: MessageFrameBridge;
  _generation: number;

  constructor(bridge: MessageFrameBridge, generation: number) {
    this._bridge = bridge;
    this._generation = generation;
  }

  /** Elements in the body matching a CSS selector, with their attributes and text. */
  querySelectorAll(selector: string): Promise<MessageBodyElement[]> {
    return this._bridge.request({ op: 'query', selector }, this._generation);
  }

  /** Sets attributes (null removes one). Event handler attributes are refused. */
  setAttributes(ref: number, attributes: { [name: string]: string | null }): Promise<void> {
    return this._bridge.request({ op: 'setAttributes', ref, attributes }, this._generation);
  }

  /**
   * Adds an element next to or inside `ref` and returns its ref. Scripts, frames, forms,
   * styles and document-level elements are refused.
   */
  insertElement(
    ref: number,
    position: InsertPosition,
    tagName: string,
    attributes: { [name: string]: string } = {},
    text = ''
  ): Promise<number> {
    return this._bridge.request(
      { op: 'insert', ref, position, tagName, attributes, text },
      this._generation
    );
  }

  /** The element's box in app window coordinates (for popovers). */
  async getBoundingClientRect(ref: number): Promise<MessageBodyRect> {
    const rect = await this._bridge.request<FrameRect>({ op: 'rect', ref }, this._generation);
    return this._bridge.toWindowRect(rect);
  }

  /** Calls `cb` with the element's box when it is clicked or pressed. */
  addEventListener(
    ref: number,
    event: 'click' | 'mousedown',
    cb: (e: { rect: MessageBodyRect }) => void
  ): Promise<void> {
    return this._bridge.listen(this._generation, ref, event, cb).then(() => undefined);
  }
}
