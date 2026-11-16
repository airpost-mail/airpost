// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// Drives the composer the way Chromium does, with the real ComposerEditor's slate-react
// configuration: its props (plugins, schema, onKeyDown, onPaste, onChange) go through
// slate-react's ReactPlugin, which wraps them in its Before/After plugins exactly as a
// mounted <Editor> does. Nothing is mounted, so the result depends only on the plugin stack.
//
// A typed character is keydown, then (unless a handler cancelled it) the textInput that React
// turns into onBeforeInput, then keyup. A handler that changes the value schedules slate's
// flush in a microtask; the browser runs it before the next event, so the driver flushes
// after every event.

// Before slate-react, as in composer-editor.tsx: slate-react reads the environment on import.
import '../../src/components/composer-editor/patch-chrome-ime';
import { Editor, Value } from 'slate';
import * as SlateReact from 'slate-react';
import { ComposerEditor } from '../../src/components/composer-editor/composer-editor';
import { ComposerEditorPlaintext } from '../../src/components/composer-editor/plaintext-editor';
import { convertToHTML, convertToPlainText } from '../../src/components/composer-editor/conversion';

// Exported by slate-react at runtime but missing from its type declarations.
const ReactPlugin: (props: any) => any = (SlateReact as any).ReactPlugin;
const SlateEditorComponent = SlateReact.Editor;

type BlockSpec =
  | string
  | { type: string; text?: string; nodes?: BlockSpec[]; data?: { [key: string]: any } };

const KEY_CODES: { [key: string]: number } = {
  Backspace: 8,
  Tab: 9,
  Enter: 13,
  Escape: 27,
  ' ': 32,
  ArrowLeft: 37,
  ArrowUp: 38,
  ArrowRight: 39,
  ArrowDown: 40,
  Delete: 46,
  Process: 229,
};

// A key that types a character: one code point, so an astral emoji counts as one.
function isCharacter(key: string) {
  return [...key].length === 1;
}

const SHIFTED = '~!@#$%^&*()_+{}|:"<>?';

export interface KeyOptions {
  shiftKey?: boolean;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  isComposing?: boolean;
}

export function keyEvent(key: string, opts: KeyOptions = {}) {
  const keyCode = KEY_CODES[key] ?? (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
  let prevented = false;
  let stopped = false;
  return {
    key,
    keyCode,
    which: keyCode,
    charCode: 0,
    repeat: false,
    shiftKey: opts.shiftKey ?? (isCharacter(key) && (SHIFTED.includes(key) || /[A-Z]/.test(key))),
    metaKey: !!opts.metaKey,
    ctrlKey: !!opts.ctrlKey,
    altKey: !!opts.altKey,
    target: null,
    nativeEvent: { isComposing: !!opts.isComposing },
    preventDefault() {
      prevented = true;
    },
    isDefaultPrevented() {
      return prevented;
    },
    get defaultPrevented() {
      return prevented;
    },
    stopPropagation() {
      stopped = true;
    },
    isPropagationStopped() {
      return stopped;
    },
    persist() {},
  };
}

function textInputEvent(data: string) {
  // React's polyfill of beforeinput: a synthetic event (it has a nativeEvent).
  const event = keyEvent('');
  return Object.assign(event, { data, nativeEvent: { data } });
}

function blockJSON(spec: BlockSpec): any {
  if (typeof spec === 'string') {
    return { object: 'block', type: 'div', nodes: [{ object: 'text', leaves: [{ text: spec }] }] };
  }
  return {
    object: 'block',
    type: spec.type,
    data: spec.data || {},
    nodes: spec.nodes
      ? spec.nodes.map(blockJSON)
      : [{ object: 'text', leaves: [{ text: spec.text || '' }] }],
  };
}

// A value from block specs: a string is a div, `{type, text}` a leaf block of that type and
// `{type, nodes}` a container (`ul_list` > `list_item` > div, `blockquote` > div).
export function valueFromBlocks(blocks: BlockSpec[] = ['']) {
  return Value.fromJSON({ document: { nodes: blocks.map(blockJSON) } } as any);
}

function findElement(node: any, type: any): any {
  if (!node || typeof node !== 'object') return null;
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findElement(child, type);
      if (found) return found;
    }
    return null;
  }
  if (node.type === type) return node;
  return node.props ? findElement(node.props.children, type) : null;
}

export class ComposerTyping {
  composer: ComposerEditor;
  editor: Editor;
  changes = 0;

  constructor(
    value: Value = valueFromBlocks(),
    { propsForPlugins = {} }: { propsForPlugins?: any } = {}
  ) {
    this.composer = new ComposerEditor({
      value,
      propsForPlugins,
      onChange: (change) => {
        this.changes += 1;
        // As a re-render does: slate-react hands the controlled value back to its controller.
        if (change.value !== this.editor.value) this.editor.setValue(change.value);
      },
    });
    this.composer._mounted = true;

    const element = findElement(this.composer.render(), SlateEditorComponent);
    const props = { ...element.props, value };
    this.editor = new Editor({
      plugins: [ReactPlugin(props) as any],
      value,
      onChange: (change) => this.composer.onChange(change as any),
    });
    // Plugins read the slate-react component's props (propsForPlugins) from the editor.
    (this.editor as any).props = props;
    this.editor.focus();
    this.moveToEnd();
    this.flush();
  }

  get value() {
    return this.editor.value;
  }

  flush() {
    if ((this.editor as any).tmp.flushing) this.editor.flush();
  }

  run(handler: string, event: any) {
    this.editor.run(handler, event);
    this.flush();
    return event;
  }

  moveToEnd() {
    const last = this.value.document.getLastText();
    if (last) this.editor.moveToEndOfNode(last);
    this.flush();
    return this;
  }

  // Puts the cursor in the leaf block at `blockIndex` (document order) at `offset`.
  moveTo(blockIndex: number, offset: number) {
    const block = this.value.document.getLeafBlocksAtRange(this.allRange()).get(blockIndex);
    this.editor.moveTo(block.getFirstText().key, offset);
    this.flush();
    return this;
  }

  allRange() {
    const { document } = this.value;
    return {
      anchor: { key: document.getFirstText().key, offset: 0 },
      focus: { key: document.getLastText().key, offset: document.getLastText().text.length },
    } as any;
  }

  keyDown(key: string, opts: KeyOptions = {}) {
    return this.run('onKeyDown', keyEvent(key, opts));
  }

  keyUp(key: string, opts: KeyOptions = {}) {
    return this.run('onKeyUp', keyEvent(key, opts));
  }

  textInput(data: string) {
    return this.run('onBeforeInput', textInputEvent(data));
  }

  // One keystroke as Chromium delivers it. Returns whether a handler cancelled the keydown.
  press(key: string, opts: KeyOptions = {}) {
    const down = this.keyDown(key, opts);
    const printable = isCharacter(key) && !opts.metaKey && !opts.ctrlKey;
    if (printable && !down.isDefaultPrevented()) this.textInput(key);
    this.keyUp(key, opts);
    return down.isDefaultPrevented();
  }

  // Types `text` one key at a time. Named keys go in braces: "a{Enter}b{Backspace}".
  type(text: string) {
    for (const key of splitKeys(text)) this.press(key);
    return this;
  }

  // Types `text` as a fast typist does: each key's keyup arrives after the next key's keydown
  // and textInput.
  typeRollover(text: string) {
    const keys = splitKeys(text);
    let pendingUp: string | null = null;
    for (const key of keys) {
      const down = this.keyDown(key);
      if (isCharacter(key) && !down.isDefaultPrevented()) this.textInput(key);
      if (pendingUp) this.keyUp(pendingUp);
      pendingUp = key;
    }
    if (pendingUp) this.keyUp(pendingUp);
    return this;
  }

  // An IME composition that commits `committed`, with Chromium's event order.
  compose(steps: string[], committed: string) {
    this.run('onCompositionStart', { ...keyEvent(''), data: '' });
    for (let i = 0; i < steps.length; i++) {
      this.keyDown('Process', { isComposing: true });
      this.keyUp('Process', { isComposing: true });
    }
    this.textInput(committed);
    this.run('onCompositionEnd', { ...keyEvent(''), data: committed });
    return this;
  }

  pastePlainText(text: string) {
    const data: { [type: string]: string } = { 'text/plain': text };
    const event = Object.assign(keyEvent(''), {
      clipboardData: {
        types: Object.keys(data),
        items: [],
        getData: (type: string) => data[type] || '',
      },
    });
    return this.run('onPaste', event);
  }

  // The text of each leaf block.
  blocks() {
    return this.value.document
      .getLeafBlocksAtRange(this.allRange())
      .toArray()
      .map((b) => b.text);
  }

  text() {
    return convertToPlainText(this.value);
  }

  html() {
    return convertToHTML(this.value);
  }

  blockTypes() {
    return this.value.document
      .getLeafBlocksAtRange(this.allRange())
      .toArray()
      .map((b) => {
        const ancestors = this.value.document.getAncestors(b.key)!.toArray().slice(1);
        return [...ancestors.map((a: any) => a.type), b.type].join('>');
      });
  }

  // Where the cursor is: the leaf block's index and the offset in that block's text.
  cursor() {
    const { selection, document } = this.value;
    const blocks = document.getLeafBlocksAtRange(this.allRange()).toArray();
    const block = document.getClosestBlock(selection.focus.key)!;
    const index = blocks.findIndex((b) => b.key === block.key);
    const offset = block.getOffset(selection.focus.key) + selection.focus.offset;
    return { block: index, offset, collapsed: selection.isCollapsed };
  }

  marksAt(blockIndex: number, offset: number) {
    const block = this.value.document.getLeafBlocksAtRange(this.allRange()).get(blockIndex);
    return block
      .getMarksAtRange({
        anchor: { key: block.getFirstText().key, offset },
        focus: { key: block.getFirstText().key, offset: offset + 1 },
      } as any)
      .toArray()
      .map((m) => m.type)
      .sort();
  }

  inlines() {
    return this.value.document
      .getInlines()
      .toArray()
      .map((i) => ({ type: i.type, data: i.data.toJS() }));
  }
}

const NAMED_KEYS = Object.keys(KEY_CODES).filter((k) => k.length > 1);

// Splits typed text into keys: one per character (code point), with the named keys written
// in braces, eg "a{Enter}b". Braces around anything else are typed.
export function splitKeys(text: string) {
  const keys: string[] = [];
  const re = new RegExp(`\\{(${NAMED_KEYS.join('|')})\\}|([\\s\\S])`, 'gu');
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) keys.push(m[1] || m[2]);
  return keys;
}

// A stand-in for the <textarea> the plaintext composer edits, with the selection API its
// handlers use.
class FakeTextarea {
  value = '';
  selectionStart = 0;
  selectionEnd = 0;
  selectionDirection = 'forward';
  setSelectionRange(start: number | null, end: number | null) {
    this.selectionStart = start ?? 0;
    this.selectionEnd = end ?? 0;
  }
}

function textarea(): any {
  return typeof document !== 'undefined' ? document.createElement('textarea') : new FakeTextarea();
}

// The plaintext composer typed into: the browser edits the textarea, then React's onChange
// and onKeyUp run.
export class PlaintextTyping {
  composer: ComposerEditorPlaintext;
  el: any = textarea();
  value: string;

  constructor(value = '') {
    this.value = value;
    this.composer = new ComposerEditorPlaintext({
      value,
      propsForPlugins: {},
      onChange: (next) => (this.value = next),
    });
    this.el.value = value;
    this.el.setSelectionRange(value.length, value.length);
  }

  private edit(nextValue: string, cursor: number) {
    this.el.value = nextValue;
    this.el.setSelectionRange(cursor, cursor);
    this.composer.onChange({ target: this.el } as any);
  }

  press(key: string) {
    const { value, selectionStart: start, selectionEnd: end } = this.el;
    if (key === 'Backspace') {
      const from = start === end ? Math.max(0, start - 1) : start;
      this.edit(value.slice(0, from) + value.slice(end), from);
    } else {
      const ch = key === 'Enter' ? '\n' : key;
      this.edit(value.slice(0, start) + ch + value.slice(end), start + ch.length);
    }
    this.composer.onKeyUp({ key, target: this.el, preventDefault() {} } as any);
  }

  type(text: string) {
    for (const key of splitKeys(text)) this.press(key);
    return this;
  }

  cursor() {
    return this.el.selectionStart;
  }
}
