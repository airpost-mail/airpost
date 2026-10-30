import React from 'react';
import * as NodeEmoji from 'node-emoji';
import * as Actions from '../../data/actions';
import { Editor, Mark, Value } from 'slate';
import { lazyRequire } from '../../data/lazy-require';
import {
  Rule,
  ComposerEditorPlugin,
  ComposerRenderNodeProps,
  ComposerEditorPluginTopLevelComponentProps,
  ComposerEditorPluginToolbarComponentProps,
} from './types';
import EmojiToolbarPopover from './emoji-toolbar-popover';
import { safeActiveMarks } from './toolbar-component-factories';

const emojiNameToImageTable = lazyRequire((): { [name: string]: string } =>
  require('./emoji-name-to-image-table')
);

// A Mark type used to store data on the `:emoji` characters that
// cause the floating picker to be displayed.
const EMOJI_TYPING_TYPE = 'emojitype';

// An Inline type used for actual emoji in the editor, which we
// display as images rather than text characters for visual consistency
// across platforms (textual emoji suck on Windows and Linux)
export const EMOJI_TYPE = 'emoji';

// The size of the list of floating emoji options
const MAX_EMOJI_SUGGESTIONS = 6;

// Cache all emoji names for efficient autocomplete lookup
let allEmojiNames: string[] | null = null;

function getAllEmojiNames(): string[] {
  if (!allEmojiNames) {
    // node-emoji v2: search() returns { name, emoji } objects where `name` is the identifier.
    allEmojiNames = NodeEmoji.search('')
      .map((e) => e.name)
      .sort();
  }
  return allEmojiNames;
}

/* Returns emoji names that start with the provided text (used by the typeahead). */
export function getEmojiSuggestions(word: string) {
  const emojiOptions = [];
  const emojiNames = getAllEmojiNames();
  for (const emojiName of emojiNames) {
    if (word === emojiName.substring(0, word.length)) {
      emojiOptions.push(emojiName);
    }
  }
  return emojiOptions;
}

/* Returns emoji names containing the provided text anywhere (used by the toolbar popover). */
export function searchEmojiNames(word: string) {
  return getAllEmojiNames().filter((name) => name.includes(word));
}

export function getEmojiImagePath(emojiname: string) {
  return `images/composer-emoji/twitter/${emojiNameToImageTable()[emojiname]}`;
}

function ImageBasedEmoji(props: { name: string }) {
  return (
    <img
      alt={props.name}
      draggable={false}
      src={getEmojiImagePath(props.name)}
      width="16"
      height="16"
      style={{ marginTop: '-4px', marginRight: '3px' }}
    />
  );
}

/* React Component that positions itself beneath the [data-emoji-typing] node
within the RichEditor. Note that this positioning logic fails on first render
because it assumes the [data-emoji-typing] noe will have already been rendered.
For the moment, I don't care.

This component doesn't get much in the way of props. It reads /all/ of it's
state (emoji to display, current selection, etc.) from the EMOJI_TYPING_TYPE
mark. Storing the state in the document is a bit odd but worked very well in
the previous composer editor.
*/
function FloatingEmojiPicker({ editor, value }: ComposerEditorPluginTopLevelComponentProps) {
  if (!value.selection.isFocused) return null;
  const sel = document.getSelection();
  if (!sel || !sel.rangeCount) return null;
  const range = sel.getRangeAt(0);

  const emoji = safeActiveMarks(value).find((i) => i.type === EMOJI_TYPING_TYPE);
  if (!emoji) return null;

  const picked: string = emoji.data.get('picked');
  const suggestions: string[] | undefined = emoji.data.get('suggestions');
  if (!suggestions || !suggestions.length) return null;

  const target = range.endContainer.parentElement?.closest('[data-emoji-typing]');
  if (!target) return null;

  const relativePositionedParent = target.closest('.RichEditor-content') as HTMLElement;
  const intrinsicPos = relativePositionedParent.getBoundingClientRect();
  const targetRect = target.getBoundingClientRect();
  const relativeParentW = relativePositionedParent.clientWidth;

  // Convert to parent-relative coordinates before clamping to editor bounds
  let targetLeft = targetRect.left - intrinsicPos.left;
  if (targetLeft + 150 > relativeParentW) {
    targetLeft = relativeParentW - 150;
  }

  const delta = {
    top: targetRect.top + targetRect.height - intrinsicPos.top,
    left: targetLeft,
  };

  // Don't display all the selections - just display a few before/after the
  // current selection so the user can "scroll" through them with the arrows
  let start = suggestions.indexOf(picked);
  start = start < 3 ? 0 : start - 2;
  if (start > suggestions.length - MAX_EMOJI_SUGGESTIONS) {
    start = suggestions.length - MAX_EMOJI_SUGGESTIONS;
  }
  const displayedSuggestions = suggestions.slice(start, start + MAX_EMOJI_SUGGESTIONS);

  return (
    <div
      className="emoji-picker"
      style={{ position: 'absolute', left: delta.left, top: delta.top }}
    >
      {displayedSuggestions.map((option) => (
        <button
          key={option}
          onMouseDown={(e) => swapEmojiMarkFor(editor, emoji, option)}
          className={`btn btn-icon ${picked === option && 'emoji-option'}`}
        >
          <ImageBasedEmoji name={option} /> :{option}
        </button>
      ))}
    </div>
  );
}

function renderNode(
  { node, attributes, targetIsHTML }: ComposerRenderNodeProps,
  editor: Editor | null = null,
  next: () => void = () => {}
) {
  if (node.type !== EMOJI_TYPE) {
    return next();
  }
  const name = node.data.get('name');
  return (
    <span {...attributes} data-emoji-name={name}>
      {targetIsHTML ? NodeEmoji.get(name) : <ImageBasedEmoji name={name} />}
    </span>
  );
}

function renderMark(
  {
    mark,
    children,
    targetIsHTML,
  }: {
    mark: any;
    children: any;
    targetIsHTML?: boolean;
  },
  editor: Editor | null = null,
  next: () => void = () => {}
) {
  if (mark.type === EMOJI_TYPING_TYPE) {
    // Never part of the message: serializing gives the text alone.
    return targetIsHTML ? <>{children}</> : <span data-emoji-typing={true}>{children}</span>;
  }
  return next();
}

const rules: Rule[] = [
  {
    deserialize(el, next) {
      if (el instanceof HTMLElement && el.dataset && el.dataset.emojiName) {
        return {
          type: EMOJI_TYPE,
          object: 'inline',
          data: {
            name: el.dataset.emojiName,
          },
        };
      }
    },
    serialize(obj: any, children: any) {
      if (obj.object === 'mark') {
        return renderMark({ mark: obj, children, targetIsHTML: true });
      } else if (obj.object === 'inline') {
        return renderNode({ node: obj, children, targetIsHTML: true });
      }
    },
  },
];

// The `:name` being typed: a colon at the start of the text or after whitespace, then the
// characters emoji names use, up to the cursor. A colon inside a word ("Re:", "10:30",
// "https://") never starts one, so typing it never offers or inserts an emoji.
const EMOJI_QUERY = /(?:^|\s)(:[a-z0-9_+-]*)$/;

interface EmojiQuery {
  typed: string;
  textKey: string;
  start: number;
}

export function emojiQueryAt(value: Value): EmojiQuery | null {
  const { selection, focusText } = value;
  if (!selection.isCollapsed || !focusText || focusText.key !== selection.focus.key) return null;
  const offset = selection.focus.offset;
  const match = EMOJI_QUERY.exec(focusText.text.slice(0, offset));
  if (!match) return null;
  return { typed: match[1], textKey: focusText.key, start: offset - match[1].length };
}

function emojiNameExists(name: string) {
  return !!name && getAllEmojiNames().includes(name);
}

function suggestionsFor(typed: string) {
  const name = typed.slice(1);
  if (name.length < 2 || (/^\d+$/.test(name) && name.length < 3)) return [];
  return getEmojiSuggestions(name);
}

function emojiTypingMark(value: Value) {
  return safeActiveMarks(value).find((m) => m.type === EMOJI_TYPING_TYPE);
}

function focusBlockHasEmojiTyping(value: Value) {
  return !!value.focusBlock?.getMarks().some((m) => m!.type === EMOJI_TYPING_TYPE);
}

// The typing mark only places the picker and carries its state. Removes it from every text of
// the focus block, wherever earlier typing left it.
function clearEmojiTyping(editor: Editor) {
  const block = editor.value.focusBlock;
  if (!block) return;
  block.getTexts().forEach((text) => {
    text!
      .getMarks()
      .filter((m) => m!.type === EMOJI_TYPING_TYPE)
      .forEach((mark) => editor.removeMarkByKey(text!.key, 0, text!.text.length, mark!));
  });
}

function showEmojiPicker(editor: Editor, query: EmojiQuery, suggestions: string[], picked: string) {
  const existing = emojiTypingMark(editor.value);
  if (existing?.data.get('typed') === query.typed && existing?.data.get('picked') === picked) {
    return;
  }
  clearEmojiTyping(editor);
  editor.addMarkByKey(
    query.textKey,
    query.start,
    query.typed.length,
    Mark.create({ type: EMOJI_TYPING_TYPE, data: { typed: query.typed, suggestions, picked } })
  );
}

// Replaces the `:name` before the cursor with the emoji. The range comes from the text as it
// is now, not from the picker's state, which lags a keystroke behind when keys overlap.
function insertEmoji(editor: Editor, name: string) {
  const query = emojiQueryAt(editor.value);
  clearEmojiTyping(editor);
  if (!query) return;
  editor
    .deleteBackward(query.typed.length)
    .insertInline({ object: 'inline', type: EMOJI_TYPE, data: { name } })
    .moveToStartOfNextText();
}

export function swapEmojiMarkFor(editor: Editor, emoji: Mark, picked: string) {
  if (picked) {
    insertEmoji(editor, picked);
  } else {
    clearEmojiTyping(editor);
  }
}

// The highlighted suggestion for the text as it is now.
function pickedSuggestion(value: Value, mark: Mark) {
  const query = emojiQueryAt(value);
  const suggestions = query ? suggestionsFor(query.typed) : [];
  const picked = mark.data.get('picked');
  return suggestions.includes(picked) ? picked : suggestions[0] || null;
}

// Enter and Tab take the highlighted suggestion and a closing colon takes an exact name
// (":smile:"). Space only ends the query, so the word after a colon is never replaced.
function onKeyDown(event: React.KeyboardEvent, editor: Editor, next: () => void) {
  if (event.metaKey || event.ctrlKey || event.altKey) return next();

  if (event.key === ':') {
    const name = emojiQueryAt(editor.value)?.typed.slice(1);
    if (name && emojiNameExists(name)) {
      event.preventDefault();
      insertEmoji(editor, name);
      return;
    }
    return next();
  }

  const mark = emojiTypingMark(editor.value);
  if (!mark) return next();

  if (['Enter', 'Return', 'Tab'].includes(event.key) && !event.shiftKey) {
    const picked = pickedSuggestion(editor.value, mark);
    if (!picked) return next();
    event.preventDefault();
    insertEmoji(editor, picked);
    return;
  }
  if (event.key === 'Escape') {
    const query = emojiQueryAt(editor.value);
    if (query) dismissedQueries.set(editor, query);
    clearEmojiTyping(editor);
    return next();
  }
  if (['ArrowDown', 'ArrowUp'].includes(event.key)) {
    const query = emojiQueryAt(editor.value);
    const suggestions = query ? suggestionsFor(query.typed) : [];
    if (!query || !suggestions.length) return next();
    const dir = event.key === 'ArrowDown' ? 1 : -1;
    const current = Math.max(0, suggestions.indexOf(mark.data.get('picked')));
    const picked = suggestions[(suggestions.length + current + dir) % suggestions.length];
    showEmojiPicker(editor, query, suggestions, picked);
    event.preventDefault();
    return;
  }
  return next();
}

// The query the picker was closed on with Escape, per editor. It stays closed while the user
// keeps typing that query; a new colon starts a new one.
const dismissedQueries = new WeakMap<Editor, EmojiQuery>();

function isDismissed(editor: Editor, query: EmojiQuery) {
  const dismissed = dismissedQueries.get(editor);
  return !!dismissed && dismissed.textKey === query.textKey && dismissed.start === query.start;
}

// Shows, updates or hides the picker after each key, from the text before the cursor.
function onKeyUp(event: React.KeyboardEvent, editor: Editor, next: () => void) {
  // Mid-composition the text isn't committed, and changing marks would re-render it.
  if (event.key === 'Process' || (event.nativeEvent as KeyboardEvent | undefined)?.isComposing) {
    return next();
  }
  const query = emojiQueryAt(editor.value);
  if (!query || !isDismissed(editor, query)) dismissedQueries.delete(editor);
  const suggestions = query && !isDismissed(editor, query) ? suggestionsFor(query.typed) : [];
  if (query && suggestions.length) {
    const picked = emojiTypingMark(editor.value)?.data.get('picked');
    showEmojiPicker(
      editor,
      query,
      suggestions,
      suggestions.includes(picked) ? picked : suggestions[0]
    );
  } else if (focusBlockHasEmojiTyping(editor.value)) {
    clearEmojiTyping(editor);
  }
  return next();
}

const ToolbarEmojiButton = ({ value, editor }: ComposerEditorPluginToolbarComponentProps) => {
  const onInsertEmoji = (name: string) => {
    const inline = {
      object: 'inline',
      type: EMOJI_TYPE,
      data: { name },
    };
    Actions.closePopover();
    setTimeout(() => {
      editor.insertInline(inline).moveToStartOfNextText().focus();
    }, 100);
  };

  return (
    <button
      onClick={(e) => {
        Actions.openPopover(<EmojiToolbarPopover onInsertEmoji={onInsertEmoji} />, {
          originRect: (e.target as HTMLElement).getBoundingClientRect(),
          direction: 'up',
        });
      }}
    >
      <i className="fa fa-smile-o" />
    </button>
  );
};

const plugins: ComposerEditorPlugin[] = [
  {
    toolbarComponents: [ToolbarEmojiButton],
    topLevelComponent: FloatingEmojiPicker,
    renderMark,
    renderNode,
    rules,
    onKeyDown,
    onKeyUp,
  },
];

export default plugins;
