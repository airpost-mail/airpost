import { Editor } from 'slate';
import { BLOCK_CONFIG } from './base-block-plugins';
import { MARK_CONFIG } from './base-mark-plugins';
import { autoReplaceOutside } from './auto-replace';
import { ComposerEditorPlugin } from './types';

// The delimiters open only at the start of the text or after whitespace, and the text they
// wrap neither starts nor ends with whitespace, as in Markdown: "2*3*4" and "a*b" stay as
// typed. AutoReplace deletes every capture group of a match, so the wrapped text is one group.
const SPAN = '([^*\\s](?:[^*]*[^*\\s])?)';

// Marks `text` and leaves the mark off for what is typed next, without typing anything the
// user didn't: the closing delimiter AutoReplace swallowed is the whole edit.
function insertMarked(editor: Editor, type: string, text: string) {
  editor.addMark(type).insertText(text).removeMark(type);
}

const NOT_IN_CODE = [BLOCK_CONFIG.code.type];

// AutoReplace only calls `change` once every configured pattern (before/after) has matched.
const plugins: ComposerEditorPlugin[] = [
  // **abc* + *
  autoReplaceOutside(NOT_IN_CODE, {
    trigger: '*',
    before: new RegExp(`(?:^|\\s)(\\*\\*)${SPAN}(\\*)$`),
    change: (editor: Editor, e, matches) => {
      insertMarked(editor, MARK_CONFIG.bold.type, matches.before![2]);
    },
  }),

  // *abc + * [and not another trailing *]
  autoReplaceOutside(NOT_IN_CODE, {
    trigger: '*',
    before: new RegExp(`(?:^|\\s)(\\*)${SPAN}$`),
    change: (editor: Editor, e, matches) => {
      insertMarked(editor, MARK_CONFIG.italic.type, matches.before![2]);
    },
  }),

  // * + * + abc**
  autoReplaceOutside(NOT_IN_CODE, {
    trigger: '*',
    before: /(?:^|\s)(\*)$/,
    after: /^([^*\s][^*]*)([*]{2})/,
    change: (editor: Editor, e, matches) => {
      const text = matches.after![1];
      editor
        .addMark(MARK_CONFIG.bold.type)
        .insertText(text)
        .moveAnchorForward(-text.length)
        .moveToAnchor();
    },
  }),

  // * + abc* [and not another trailing *]
  autoReplaceOutside(NOT_IN_CODE, {
    trigger: '*',
    before: /(?:^|\s)$/,
    after: /^([^*\s][^*]*)([*]{1})(?:[^*]{1}|$)/,
    change: (editor: Editor, e, matches) => {
      const text = matches.after![1];
      editor
        .addMark(MARK_CONFIG.italic.type)
        .insertText(text)
        .moveAnchorForward(-text.length)
        .moveToAnchor();
    },
  }),

  // Code Block Creation (```)
  autoReplaceOutside([...NOT_IN_CODE, BLOCK_CONFIG.list_item.type], {
    trigger: '`',
    before: /^([`]{2})$/,
    change: (editor: Editor) => {
      editor.setBlocks(BLOCK_CONFIG.code.type);
    },
  }),

  // Code Inline Creation (`a`)
  autoReplaceOutside(NOT_IN_CODE, {
    trigger: '`',
    before: /(?:^|[^`])([`]{1})([^`]+)$/,
    change: (editor: Editor, e, matches) => {
      insertMarked(editor, MARK_CONFIG.codeInline.type, matches.before![2]);
    },
  }),
];

export default plugins;
