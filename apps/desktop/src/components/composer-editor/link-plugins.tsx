import React from 'react';

import { Mark, Editor } from 'slate';
import RegExpUtils from '../../utils/regexp';

import { BuildMarkButtonWithValuePicker, safeActiveMarks } from './toolbar-component-factories';
import { BLOCK_CONFIG } from './base-block-plugins';
import { isInBlockOfType } from './auto-replace';
import { ComposerEditorPlugin, Rule } from './types';

export const LINK_TYPE = 'link';

function onPaste(event: React.ClipboardEvent, editor: Editor, next: () => void) {
  const html = event.clipboardData.getData('text/html');
  const plain = event.clipboardData.getData('text/plain');
  const regex = RegExpUtils.urlRegex({ matchStartOfString: true, matchTailOfString: true });
  if (!html && plain && plain.match(regex)) {
    const mark = Mark.create({ type: LINK_TYPE, data: { href: plain } });
    editor.addMark(mark).insertText(plain).removeMark(mark);
  } else {
    return next();
  }
}

const LinkPatterns = [
  {
    regex: () =>
      RegExpUtils.emailRegex({ requireStartOrWhitespace: true, matchTailOfString: true }),
    href: (text: string) => `mailto:${text}`,
  },
  {
    regex: () => RegExpUtils.urlRegex({ matchTailOfString: true }),
    // "example.com" alone would be a relative URL, which goes nowhere from an email.
    href: (text: string) => (/^[a-z][a-z0-9+.-]*:/i.test(text) ? text : `http://${text}`),
  },
];

// Makes the URL or email address that ends at the cursor a link, in place.
function linkTextBeforeCursor(editor: Editor) {
  const { selection, focusText, document } = editor.value;
  if (!selection.isCollapsed || !focusText || focusText.key !== selection.focus.key) return;
  if (isInBlockOfType(editor.value, [BLOCK_CONFIG.code.type])) return;

  const offset = selection.focus.offset;
  const before = focusText.text.slice(0, offset);
  for (const { regex, href } of LinkPatterns) {
    // Both patterns match only at the end, so there is at most one match.
    const text = (before.match(regex()) || [])[0]?.trim();
    if (!text) continue;
    const start = offset - text.length;
    const range = { anchor: { key: focusText.key, offset: start }, focus: selection.focus };
    const linked = document.getMarksAtRange(range as any).some((m) => m!.type === LINK_TYPE);
    if (!linked) {
      editor.addMarkByKey(
        focusText.key,
        start,
        text.length,
        Mark.create({ type: LINK_TYPE, data: { href: href(text) } })
      );
    }
    return;
  }
}

function renderMark(
  {
    mark,
    children,
    targetIsHTML,
  }: { mark: Mark; children: React.ReactNode; targetIsHTML?: boolean },
  editor: Editor | null = null,
  next: () => void = () => {}
) {
  if (mark.type !== LINK_TYPE) {
    return next();
  }
  const href = mark.data.get('href');
  if (targetIsHTML) {
    return (
      <a href={href} title={href}>
        {children}
      </a>
    );
  } else {
    return (
      <span
        className="link"
        title={href}
        spellCheck={false}
        onClick={(e) => {
          if (e.ctrlKey || e.metaKey || e.altKey) {
            AppEnv.windowEventHandler.openLink({ href, metaKey: e.metaKey });
          }
        }}
      >
        {children}
      </span>
    );
  }
}

const rules: Rule[] = [
  {
    deserialize(el, next) {
      if (el.tagName.toLowerCase() === 'a') {
        return {
          object: 'mark',
          type: LINK_TYPE,
          nodes: next(el.childNodes),
          data: {
            href: el.getAttribute('href'),
          },
        };
      }
    },
    serialize(obj: any, children: any) {
      if (obj.object !== 'mark') return;
      return renderMark({ mark: obj, children, targetIsHTML: true });
    },
  },
];

const TriggerKeys = [' ', 'Enter', 'Return'];

const BaseLinkPlugin: ComposerEditorPlugin = {
  toolbarComponents: [
    BuildMarkButtonWithValuePicker({
      type: LINK_TYPE,
      field: 'href',
      iconClassOn: 'fa fa-link',
      iconClassOff: 'fa fa-link',
      placeholder: 'http://',
    }),
  ],
  onPaste: onPaste as any,
  // Space and Enter after a URL or email address link it where it stands and then do what
  // they always do, so Enter still splits the block, adds a list item or leaves a quote.
  onKeyDown: function onKeyDown(event: React.KeyboardEvent, editor: Editor, next: () => void) {
    if (!TriggerKeys.includes(event.key) || event.metaKey || event.ctrlKey || event.altKey) {
      return next();
    }
    linkTextBeforeCursor(editor);
    // Space and Enter always end a link.
    const mark = safeActiveMarks(editor.value).find((m) => m.type === LINK_TYPE);
    if (mark) {
      editor.removeMark(mark);
    }
    return next();
  },
  renderMark: ({ mark, children, targetIsHTML }, editor, next) => {
    return renderMark({ mark, children, targetIsHTML }, editor, next);
  },
  rules,
  appCommands: {
    'contenteditable:insert-link': (event, editor) => {
      // want to see a hack? here you go!
      // 1: find our container and then find the link toolbar icon - this approach
      // ensures we get the link button in the /current/ composer.
      const linkButton = (event.target as HTMLElement)
        .closest('.RichEditor-root')
        ?.querySelector('.fa.fa-link')
        ?.closest('button');

      // 2: click it.
      if (linkButton) {
        linkButton.dispatchEvent(
          new MouseEvent('mousedown', {
            view: window,
            bubbles: true,
            cancelable: true,
          })
        );
      }
      return editor;
    },
  },
};

const plugins: ComposerEditorPlugin[] = [BaseLinkPlugin];

export default plugins;
