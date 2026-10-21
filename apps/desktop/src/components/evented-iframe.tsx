/*
 * decaffeinate suggestions:
 * DS001: Remove Babel/TypeScript constructor workaround
 * DS207: Consider shorter variations of null checks
 * Full docs: https://github.com/decaffeinate/decaffeinate/blob/master/docs/suggestions.md
 */
import React from 'react';
import ReactDOM from 'react-dom';
import * as Utils from '../data/models/utils';
import { localized } from '../intl/localize';
import SearchableComponentStore from '../data/stores/searchable-component';
import IFrameSearcher from './searchable/iframe-searcher';
import url from 'url';
import _ from 'underscore';
import path from 'path';
import fs from 'fs';
import { ipcRenderer, shell } from 'electron';
import { FilesScheme, resolveFilesSchemeURL } from '../mail/files-scheme';
import { markFileFromInternet } from '../mail/attachment-safety';
import { ExternalFetch } from '../window/external-fetch';
import { FormSubmissionTarget, formSubmissionTarget } from '../mail/frame/form-submission';
import { Clipboard, ContextMenuItem, Dialogs, Menus } from '../platform/app-api';

// The bytes behind an image src in a message, for Save Image.
async function imageBytes(src: string): Promise<Buffer> {
  if (src.startsWith('data:')) {
    const comma = src.indexOf(',');
    const meta = src.slice(5, comma);
    const payload = src.slice(comma + 1);
    return meta.endsWith(';base64')
      ? Buffer.from(payload, 'base64')
      : Buffer.from(decodeURIComponent(payload));
  }
  if (src.startsWith(`${FilesScheme}:`)) {
    const filePath = resolveFilesSchemeURL(path.join(AppEnv.getConfigDirPath(), 'files'), src);
    if (!filePath) throw new Error('not an attachment');
    return fs.promises.readFile(filePath);
  }
  const response = await ExternalFetch.fetch(src);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

const ImageMimeTypes: { [ext: string]: string } = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.bmp': 'image/bmp',
  '.svg': 'image/svg+xml',
};

// Copy Image. The bytes are re-read and drawn from a data: URL because a tern-file: or remote
// image is cross-origin to the app document and would taint the canvas.
export async function copyImageToClipboard(src: string) {
  const buffer = await imageBytes(src);
  const ext = path.extname(src.split(/[?#]/)[0]).toLowerCase();
  const mime = ImageMimeTypes[ext] || 'image/png';
  const img = new Image();
  await new Promise<void>((resolve, reject) => {
    img.onload = () => resolve();
    img.onerror = () => reject(new Error('the image could not be decoded'));
    img.src = `data:${mime};base64,${buffer.toString('base64')}`;
  });
  const canvas = document.createElement('canvas');
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  // A new canvas always has a 2d context available.
  canvas.getContext('2d')!.drawImage(img, 0, 0);
  ipcRenderer.send('write-image-to-clipboard', canvas.toDataURL('image/png'));
}

let formConfirmOpen = false;

/**
 * Asks before a form from a message opens in the browser. Tern never submits forms inside
 * messages; a POST form's page opens without its fields. Resolves false without asking while
 * another form's dialog is open.
 */
export async function confirmFormSubmission({ url, method }: FormSubmissionTarget) {
  if (formConfirmOpen) {
    return false;
  }
  formConfirmOpen = true;
  try {
    const response = await Dialogs.ask(
      {
        type: 'question',
        buttons: [localized('Open in Browser'), localized('Cancel')],
        defaultId: 1,
        cancelId: 1,
        message: localized('Open this form in your browser?'),
        detail:
          method === 'post'
            ? localized(
                "Tern doesn't submit forms inside messages. Your browser will open %@ without the form's contents.",
                url
              )
            : localized(
                "Tern doesn't submit forms inside messages. Your browser will open %@.",
                url
              ),
      },
      'window'
    );
    return response === 0;
  } finally {
    formConfirmOpen = false;
  }
}

type EventedIFrameProps = {
  searchable?: boolean;
  onResize?: (...args: any[]) => any;
};
// `ref` on <EventedIFrame> is the component instance, not the iframe element.
type EventedIFrameAllProps = EventedIFrameProps & Omit<React.HTMLProps<HTMLIFrameElement>, 'ref'>;

/*
Public: EventedIFrame is a thin wrapper around the DOM's standard `<iframe>` element.
You should always use EventedIFrame, because it provides important event hooks that
ensure keyboard and mouse events are properly delivered to the application when
fired within iFrames.

```
<div className="file-frame-container">
  <EventedIFrame src={src} />
  <Spinner visible={!@state.ready} />
</div>
```

Any `props` added to the <EventedIFrame> are passed to the iFrame it renders.

Section: Component Kit
*/
export class EventedIFrame extends React.Component<EventedIFrameAllProps> {
  static displayName = 'EventedIFrame';

  static ownPropKeys = ['searchable', 'onResize'];

  // Both are set in componentDidMount when `searchable`, and only used then.
  _regionId!: string;
  _searchUsub!: () => void;

  render() {
    const otherProps = Utils.fastOmit(this.props, EventedIFrame.ownPropKeys);
    return <iframe title="iframe" seamless {...otherProps} />;
  }

  componentDidMount() {
    if (this.props.searchable) {
      this._regionId = Utils.generateTempId();
      this._searchUsub = SearchableComponentStore.listen(this._onSearchableStoreChange);
      SearchableComponentStore.registerSearchRegion(
        this._regionId,
        ReactDOM.findDOMNode(this) as HTMLElement
      );
    }
    this._subscribeToIFrameEvents();
  }

  componentWillUnmount() {
    this._unsubscribeFromIFrameEvents();
    if (this.props.searchable) {
      this._searchUsub();
      SearchableComponentStore.unregisterSearchRegion(this._regionId);
    }
  }

  componentDidUpdate() {
    if (this.props.searchable) {
      SearchableComponentStore.registerSearchRegion(
        this._regionId,
        ReactDOM.findDOMNode(this) as HTMLElement
      );
    }
  }

  shouldComponentUpdate(nextProps: EventedIFrameAllProps, nextState: Record<string, unknown>) {
    return !Utils.isEqualReact(nextProps, this.props) || !Utils.isEqualReact(nextState, this.state);
  }

  /*
  Public: Call this method if you replace the contents of the iframe's document.
  This allows {EventedIframe} to re-attach it's event listeners.
  */
  didReplaceDocument() {
    this._unsubscribeFromIFrameEvents();
    this._subscribeToIFrameEvents();
  }

  setHeightQuietly(height: number) {
    const el = ReactDOM.findDOMNode(this) as HTMLIFrameElement;
    if (el.style.height !== `${height}px`) {
      el.style.height = `${height}px`;
    }
  }

  lastSearchIndex?: number | null;
  lastSearchTerm?: string | null;

  _onSearchableStoreChange = () => {
    if (!this.props.searchable) {
      return;
    }
    const node = ReactDOM.findDOMNode(this) as HTMLIFrameElement;
    const doc =
      (node.contentDocument != null ? node.contentDocument.body : undefined) != null
        ? node.contentDocument != null
          ? node.contentDocument.body
          : undefined
        : node.contentDocument;
    const searchIndex = SearchableComponentStore.getCurrentRegionIndex(this._regionId);
    const { searchTerm } = SearchableComponentStore.getCurrentSearchData();
    if (this.lastSearchIndex !== searchIndex || this.lastSearchTerm !== searchTerm) {
      IFrameSearcher.highlightSearchInDocument(this._regionId, searchTerm, doc, searchIndex);
    }
    this.lastSearchIndex = searchIndex;
    this.lastSearchTerm = searchTerm;
  };

  _unsubscribeFromIFrameEvents() {
    const node = ReactDOM.findDOMNode(this) as HTMLIFrameElement;
    const doc = node.contentDocument;
    if (!doc) {
      return;
    }
    doc.removeEventListener('click', this._onIFrameClick);
    doc.removeEventListener('submit', this._onIFrameSubmit, true);
    doc.removeEventListener('keydown', this._onIFrameKeyEvent);
    doc.removeEventListener('keypress', this._onIFrameKeyEvent);
    doc.removeEventListener('keyup', this._onIFrameKeyEvent);
    doc.removeEventListener('mousedown', this._onIFrameMouseEvent);
    doc.removeEventListener('mousemove', this._onIFrameMouseEvent);
    doc.removeEventListener('mouseup', this._onIFrameMouseEvent);
    doc.removeEventListener('contextmenu', this._onIFrameContextualMenu);
    if (node.contentWindow) {
      node.contentWindow.removeEventListener('focus', this._onIFrameFocus);
      node.contentWindow.removeEventListener('blur', this._onIFrameBlur);
    }
  }

  _subscribeToIFrameEvents() {
    const node = ReactDOM.findDOMNode(this) as HTMLIFrameElement;
    const doc = node.contentDocument;
    _.defer(() => {
      // Null when the frame is cross-origin; there is nothing to listen to then.
      if (!doc) return;
      doc.addEventListener('click', this._onIFrameClick);
      doc.addEventListener('submit', this._onIFrameSubmit, true);
      doc.addEventListener('keydown', this._onIFrameKeyEvent);
      doc.addEventListener('keypress', this._onIFrameKeyEvent);
      doc.addEventListener('keyup', this._onIFrameKeyEvent);
      doc.addEventListener('mousedown', this._onIFrameMouseEvent);
      doc.addEventListener('mousemove', this._onIFrameMouseEvent);
      doc.addEventListener('mouseup', this._onIFrameMouseEvent);
      doc.addEventListener('contextmenu', this._onIFrameContextualMenu);
      if (node.contentWindow) {
        node.contentWindow.addEventListener('focus', this._onIFrameFocus);
        node.contentWindow.addEventListener('blur', this._onIFrameBlur);
      }
    });
  }

  _getContainingTarget(event: MouseEvent, options: { with: string }): HTMLElement | null {
    let target = event.target as HTMLElement | null;
    while (target != null && target !== (document as unknown) && target !== (window as unknown)) {
      if (target.getAttribute(options.with) != null) {
        return target;
      }
      target = target.parentElement;
    }
    return null;
  }

  _onIFrameBlur = (_event: FocusEvent) => {
    const node = ReactDOM.findDOMNode(this) as HTMLIFrameElement;
    node.contentWindow?.getSelection()?.empty();
  };

  _onIFrameFocus = (_event: FocusEvent) => {
    window.getSelection()?.empty();
  };

  // The iFrame captures events that take place over it, which causes some
  // interesting behaviors. For example, when you drag and release over the
  // iFrame, the mouseup never fires in the parent window.
  _onIFrameClick = (e: MouseEvent) => {
    e.stopPropagation();
    const target = this._getContainingTarget(e, { with: 'href' });
    if (target) {
      // Sometimes urls can have relative, malformed, or malicious href
      // targets. We test the existence of a valid RFC 3986 scheme and make
      // sure the protocol isn't blacklisted. We never allow `file:` links
      // through.
      // _getContainingTarget only returns elements that have an href.
      let rawHref = target.getAttribute('href')!;

      if (this._isBlacklistedHref(rawHref)) {
        e.preventDefault();
        return;
      }

      if (!url.parse(rawHref).protocol) {
        // Check for protocol-relative uri's
        if (new RegExp(/^\/\//).test(rawHref)) {
          rawHref = `https:${rawHref}`;
        } else {
          rawHref = `http://${rawHref}`;
        }
        target.setAttribute('href', rawHref);
      }

      e.preventDefault();

      // It's important to send the raw `href` here instead of the target.
      // The `target` comes from the document context of the iframe, which
      // as of Electron 0.36.9, has different constructor function objects
      // in memory than the main execution context. This means that code
      // like `e.target instanceof Element` will erroneously return false
      // since the `e.target.constructor` and the `Element` function are
      // created in different contexts.
      AppEnv.windowEventHandler.openLink({ href: rawHref, metaKey: e.metaKey });
    }
  };

  // Forms in mail never submit in the frame, where the response would load inside Tern
  // (a fake "sign in again" page, say). After a confirm, a GET form opens with its fields in
  // the browser through the same path as links; a POST form's page opens without its fields.
  // The message CSP's form-action 'none' and AppWindow's will-frame-navigate guard
  // stop the submission too if this listener isn't attached yet.
  _onIFrameSubmit = async (e: SubmitEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const submission = formSubmissionTarget(e.target as HTMLFormElement, e.submitter);
    if (!submission || !(await this._confirmFormSubmission(submission))) {
      return;
    }
    AppEnv.windowEventHandler.openLink({ href: submission.url });
  };

  _confirmFormSubmission(submission: FormSubmissionTarget) {
    return confirmFormSubmission(submission);
  }

  _isBlacklistedHref(href: string) {
    return new RegExp(/^file:/i).test(href);
  }

  _onIFrameMouseEvent = (event: MouseEvent) => {
    const node = ReactDOM.findDOMNode(this) as HTMLIFrameElement;
    const nodeRect = node.getBoundingClientRect();

    const eventAttrs: { [key: string]: unknown } = {};
    for (const [key, value] of Object.entries(event)) {
      if (['webkitMovementX', 'webkitMovementY'].includes(key)) {
        continue;
      }
      eventAttrs[key] = value;
    }

    node.dispatchEvent(
      new MouseEvent(event.type, {
        ...eventAttrs,
        bubbles: true,
        clientX: event.clientX + nodeRect.left,
        clientY: event.clientY + nodeRect.top,
        screenX: event.screenX + nodeRect.left,
        screenY: event.screenY + nodeRect.top,
      })
    );
  };

  _onIFrameKeyEvent = (event: KeyboardEvent) => {
    if (event.metaKey || event.altKey || event.ctrlKey) {
      return;
    }

    const attrs = [
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
    const eventInit = Object.assign({ bubbles: true }, _.pick(event, attrs));
    const eventInParentDoc = new KeyboardEvent(event.type, eventInit);

    Object.defineProperty(eventInParentDoc, 'which', { value: event.which });

    (ReactDOM.findDOMNode(this) as HTMLIFrameElement).dispatchEvent(eventInParentDoc);
  };

  _onIFrameContextualMenu = (event: MouseEvent) => {
    event.preventDefault();

    const linkTarget = this._getContainingTarget(event, { with: 'href' });
    const imageTarget = this._getContainingTarget(event, { with: 'src' });

    let text = '';
    // This listener is attached to contentDocument, so it is set.
    const selection = (
      ReactDOM.findDOMNode(this) as HTMLIFrameElement
    ).contentDocument!.getSelection();
    if (selection && selection.rangeCount > 0) {
      text = selection.getRangeAt(0).toString();
    }
    if (!text || text.length === 0) {
      text = (linkTarget != null ? linkTarget : (event.target as HTMLElement)).innerText;
    }

    showFrameContextMenu({
      href: linkTarget ? linkTarget.getAttribute('href') : null,
      src: imageTarget ? imageTarget.getAttribute('src') : null,
      text: text || '',
    });
  };
}

/**
 * The context menu for a link, an image or text in a message body: Open and Copy Link,
 * Save and Copy Image, Copy, Search Google and Look Up. Message frames are cross-origin, so
 * they send these three values instead of their DOM.
 */
export function showFrameContextMenu(contents: {
  href: string | null;
  src: string | null;
  text: string;
}) {
  const { href, src } = contents;
  const menu: ContextMenuItem[] = [];

  // Menu actions for links
  if (href) {
    if (href.startsWith('mailto')) {
      menu.push({
        label: localized('Compose New Message'),
        click() {
          AppEnv.windowEventHandler.openLink({ href });
        },
      });
      menu.push({
        label: localized('Copy Email Address'),
        click() {
          Clipboard.writeText(href.split('mailto:').pop() ?? '');
        },
      });
    } else {
      menu.push({
        label: localized('Open Link'),
        click() {
          AppEnv.windowEventHandler.openLink({ href });
        },
      });
      menu.push({
        label: localized('Copy Link Address'),
        click() {
          Clipboard.writeText(href);
        },
      });
    }
    menu.push({ type: 'separator' });
  }

  // Menu actions for images
  if (src) {
    const srcFilename = path.basename(src);
    menu.push({
      label: localized('Save Image') + '...',
      click() {
        AppEnv.showSaveDialog({ defaultPath: srcFilename }, function (path: string | undefined) {
          if (!path) {
            return;
          }
          imageBytes(src)
            .then((buffer) => {
              fs.writeFile(path, buffer, async (err) => {
                if (err) {
                  console.error(`Save Image failed: ${err}`);
                  return;
                }
                await markFileFromInternet(path);
                shell.showItemInFolder(path);
              });
            })
            .catch((err) => console.error(`Save Image failed: ${err}`));
        });
      },
    });
    menu.push({
      label: localized('Copy Image'),
      click() {
        copyImageToClipboard(src).catch((err) => console.error(`Copy Image failed: ${err}`));
      },
    });
    menu.push({ type: 'separator' });
  }

  // Menu actions for text
  const text = (contents.text || '').trim();

  if (text.length > 0) {
    let textPreview;
    if (text.length > 45) {
      textPreview = text.substr(0, 42) + '...';
    } else {
      textPreview = text;
    }
    menu.push({
      label: localized('Copy'),
      click() {
        Clipboard.writeText(text);
      },
    });
    menu.push({
      label: localized(`Search Google for '%@'`, textPreview),
      click() {
        shell.openExternal(`https://www.google.com/search?q=${encodeURIComponent(text)}`);
      },
    });
    if (process.platform === 'darwin') {
      menu.push({
        label: localized(`Look Up “%@”`, textPreview),
        click() {
          AppEnv.getCurrentWindow().showDefinitionForSelection();
        },
      });
    }
  }

  if (process.platform === 'darwin') {
    menu.push({ type: 'separator' });
  }
  // Services menu appears here automatically

  Menus.popup(menu);
}
