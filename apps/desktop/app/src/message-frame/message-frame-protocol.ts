// Message bodies render in a frame served by the main process on this scheme
// (browser/message-frame-protocol-handler.ts). The frame is sandboxed without
// allow-same-origin, so it is cross-origin to the app window and Chromium puts it in its
// own sandboxed renderer process, without Node. The app window and the frame talk only
// through postMessage, with the messages below.
export const MessageFrameScheme = 'tern-message';
export const MessageFrameOrigin = `${MessageFrameScheme}://frame`;
export const MessageFramePagePath = '/message.html';
export const MessageFrameScriptPath = '/runtime.js';
export const MessageFrameScriptURL = `${MessageFrameOrigin}${MessageFrameScriptPath}`;

// The sandbox the app window gives the frame. allow-forms only so that `submit` fires and the
// frame can hand the form to the app (the CSP's form-action 'none' still blocks it).
export const MessageFrameSandbox = 'allow-scripts allow-forms';

const RemoteImagesParam = 'remote-images';

/** The frame page. Its CSP (sent as a header) allows remote images only when asked. */
export function messageFrameURL({ remoteImages }: { remoteImages: boolean }) {
  return `${MessageFrameOrigin}${MessageFramePagePath}?${RemoteImagesParam}=${
    remoteImages ? 1 : 0
  }`;
}

/** Whether `url` is one of the two frame page URLs `messageFrameURL` makes. */
export function isMessageFrameURL(url: string) {
  return (
    url === messageFrameURL({ remoteImages: false }) ||
    url === messageFrameURL({ remoteImages: true })
  );
}

export function remoteImagesFromFrameURL(url: string) {
  return url === messageFrameURL({ remoteImages: true });
}

export interface FrameRect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface SearchMatchRect extends FrameRect {
  renderIndex: number;
}

export interface FramePalette {
  background: string;
  text: string;
  link: string;
}

export interface FrameRGBA {
  r: number;
  g: number;
  b: number;
  a: number;
}

export interface RenderMessage {
  type: 'render';
  generation: number;
  html: string;
  plaintext: boolean;
  themeStyles: string;
  platform: string;
  restrictWidth: boolean;
  colorScheme: 'dark' | 'light';
  // Set when the email is darkened; otherwise themeBackground decides `has-background`.
  darkPalette: FramePalette | null;
  themeBackground: FrameRGBA;
}

export interface SearchMessage {
  type: 'search';
  regionId: string;
  term: string | null;
  currentIndex: number | null;
}

export type BodyOperation =
  | { op: 'content' }
  | { op: 'query'; selector: string }
  | { op: 'setAttributes'; ref: number; attributes: { [name: string]: string | null } }
  | {
      op: 'insert';
      ref: number;
      position: InsertPosition;
      tagName: string;
      attributes?: { [name: string]: string };
      text?: string;
    }
  | { op: 'rect'; ref: number }
  | { op: 'listen'; ref: number; event: 'click' | 'mousedown'; listenerId: number };

export type RequestMessage = { type: 'request'; id: number; generation: number } & BodyOperation;

export type AppToFrameMessage = RenderMessage | SearchMessage | RequestMessage;

export interface ForwardedKeyEvent {
  type: 'keydown' | 'keypress' | 'keyup';
  key: string;
  code: string;
  location: number;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  repeat: boolean;
  isComposing: boolean;
  charCode: number;
  keyCode: number;
  which: number;
}

export interface ForwardedMouseEvent {
  type: 'mousedown' | 'mousemove' | 'mouseup';
  clientX: number;
  clientY: number;
  screenX: number;
  screenY: number;
  button: number;
  buttons: number;
  detail: number;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

export type FrameToAppMessage =
  | { type: 'ready' }
  | { type: 'rendered'; generation: number }
  | { type: 'size'; height: number }
  | { type: 'link'; href: string; metaKey: boolean }
  | { type: 'form'; url: string; method: 'get' | 'post' }
  | { type: 'key'; event: ForwardedKeyEvent }
  | { type: 'mouse'; event: ForwardedMouseEvent }
  | {
      type: 'contextmenu';
      href: string | null;
      src: string | null;
      text: string;
    }
  | { type: 'focus' }
  | {
      type: 'searchResults';
      regionId: string;
      term: string | null;
      matches: SearchMatchRect[];
    }
  | { type: 'event'; generation: number; listenerId: number; rect: FrameRect }
  | { type: 'response'; id: number; result?: unknown; error?: string };
