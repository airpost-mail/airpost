// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// Typing into the composer, key by key, through the real ComposerEditor and its plugins
// (tests/support/composer-typing.ts). What the user types is what the message says: no
// character is dropped, doubled, moved or swapped for another, and the cursor stays after it.

import { ComposerTyping, PlaintextTyping, valueFromBlocks } from '../support/composer-typing';
import { ZERO_WIDTH_SPACE } from '../../src/components/composer-editor/plaintext';
import '../../src/components/composer-editor/composer-editor';

const PRINTABLE_ASCII = Array.from({ length: 95 }, (_, i) => String.fromCharCode(32 + i));

function escapeHTML(text: string) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#x27;');
}

function typed(text: string, blocks: any[] = [''], opts = {}) {
  return new ComposerTyping(valueFromBlocks(blocks), opts).type(text);
}

const list = (...items: string[]) => ({
  type: 'ul_list',
  nodes: items.map((item) => ({ type: 'list_item', nodes: [item] })),
});

describe('Typing in the composer', function composerTyping() {
  describe('its input path', () => {
    // Chromium's beforeinput (Input Events Level 2) path of slate-react 0.21 corrupts text;
    // patch-chrome-ime.ts must run before slate-react reads the environment.
    it("runs on React's textInput polyfill, not Input Events Level 2", () => {
      expect('onbeforeinput' in HTMLElement.prototype).toBe(false);
      // slate-react's own dependency, read here as slate-react sees it.
      // eslint-disable-next-line import/no-extraneous-dependencies
      expect(require('slate-dev-environment').HAS_INPUT_EVENTS_LEVEL_2).toBe(false);
    });
  });

  describe('every printable ASCII character', () => {
    for (const ch of PRINTABLE_ASCII) {
      it(`types ${JSON.stringify(ch)} as itself, alone, after a word and inside one`, () => {
        const alone = typed(ch);
        expect(alone.blocks()).toEqual([ch]);
        expect(alone.cursor()).toEqual({ block: 0, offset: 1, collapsed: true });
        expect(alone.inlines()).toEqual([]);
        if (ch !== ' ') expect(alone.html()).toBe(`<div>${escapeHTML(ch)}</div>`);

        const after = typed(ch, ['word ']);
        expect(after.blocks()).toEqual([`word ${ch}`]);
        expect(after.cursor().offset).toBe(6);

        const inside = new ComposerTyping(valueFromBlocks(['ab'])).moveTo(0, 1).type(ch);
        expect(inside.blocks()).toEqual([`a${ch}b`]);
        expect(inside.cursor().offset).toBe(2);
      });
    }

    it('types all of them in a row as they are', () => {
      const text = PRINTABLE_ASCII.join('');
      const t = typed(text);
      expect(t.blocks()).toEqual([text]);
      expect(t.cursor().offset).toBe(text.length);
      expect(t.html()).not.toContain('data-emoji-typing');
      expect(t.inlines()).toEqual([]);
    });

    it('types all of them in a row the same when keys overlap', () => {
      const text = PRINTABLE_ASCII.join('');
      expect(new ComposerTyping().typeRollover(text).blocks()).toEqual([text]);
    });
  });

  describe('colons', () => {
    const literal = [
      'Hi:',
      ':',
      ':)',
      ':-)',
      ':D',
      ':P :/',
      'Re: lunch',
      'a:b',
      'Note:ok then',
      'Location:office ',
      '10:30 ',
      'ratio 1:100 ',
      'at 12:00, ok',
      ':smile then',
      'see :ok there',
      'mailto:a',
      'C:\\Users\\me',
      'key: value',
      '::',
    ];
    for (const text of literal) {
      it(`leaves ${JSON.stringify(text)} as typed`, () => {
        const t = typed(text);
        expect(t.blocks()).toEqual([text]);
        expect(t.inlines()).toEqual([]);
        expect(t.html()).toBe(`<div>${escapeHTML(text)}</div>`);
        expect(t.cursor().offset).toBe(text.length);
      });

      it(`leaves ${JSON.stringify(text)} as typed when keys overlap`, () => {
        expect(new ComposerTyping().typeRollover(text).blocks()).toEqual([text]);
      });
    }

    it('turns :smile: into the emoji', () => {
      const t = typed('hi :smile: there');
      expect(t.blocks()).toEqual(['hi  there']);
      expect(t.inlines()).toEqual([{ type: 'emoji', data: { name: 'smile' } }]);
      // The serializer writes a space just after a closing tag as a no-break space.
      expect(t.html()).toBe('<div>hi <span data-emoji-name="smile">😄</span>\u00a0there</div>');
    });

    it('turns :name: into the emoji for any exact name, and nothing else', () => {
      const t = typed(':+1: :x: :100: :nosuchemoji:');
      expect(t.inlines().map((i) => i.data.name)).toEqual(['+1', 'x', '100']);
      expect(t.blocks()).toEqual(['   :nosuchemoji:']);
    });

    it('offers suggestions for :sm and takes the highlighted one on Enter or Tab', () => {
      const t = typed(':smi');
      const mark = t.value.activeMarks.find((m) => m.type === 'emojitype');
      expect(mark.data.get('picked')).toBe('smile');
      expect(mark.data.get('suggestions')).toContain('smirk');
      t.type('{ArrowDown}{Enter}');
      expect(t.inlines()).toEqual([{ type: 'emoji', data: { name: 'smile_cat' } }]);
      expect(t.blocks()).toEqual(['']);
      t.type('x');
      expect(t.html()).toBe('<div><span data-emoji-name="smile_cat">😸</span>x</div>');

      const tab = typed('a :smir{Tab}');
      expect(tab.inlines()).toEqual([{ type: 'emoji', data: { name: 'smirk' } }]);
      expect(tab.blocks()).toEqual(['a ']);
    });

    it('takes the suggestion for the text as typed when Enter overtakes a keyup', () => {
      const t = new ComposerTyping();
      t.type(':smi');
      t.keyDown('r');
      t.textInput('r');
      t.press('Enter');
      t.keyUp('r');
      expect(t.inlines()).toEqual([{ type: 'emoji', data: { name: 'smirk' } }]);
      expect(t.blocks()).toEqual(['']);
    });

    it('never takes a suggestion on Space, and Space is typed', () => {
      const t = typed(':smile :ok :heart ');
      expect(t.blocks()).toEqual([':smile :ok :heart ']);
      expect(t.inlines()).toEqual([]);
    });

    it('closes the suggestions on Escape until a new colon, without changing the text', () => {
      const t = typed(':smi{Escape}le');
      expect(t.blocks()).toEqual([':smile']);
      expect(t.value.document.getMarks().toArray()).toEqual([]);
      t.type('{Enter}');
      expect(t.blocks()).toEqual([':smile', '']);
      expect(t.inlines()).toEqual([]);

      t.type('{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}:smi');
      expect(t.blocks()).toEqual([':smi']);
      expect(t.value.activeMarks.some((m) => m.type === 'emojitype')).toBe(true);
    });

    it('never sends the suggestion state as part of the message', () => {
      const t = typed('see :smi');
      expect(t.value.document.getMarks().some((m) => m.type === 'emojitype')).toBe(true);
      expect(t.html()).toBe('<div>see :smi</div>');
      t.type('{Backspace}{Backspace}{Backspace}{Backspace}x');
      expect(t.html()).toBe('<div>see x</div>');
      expect(t.value.document.getMarks().toArray()).toEqual([]);
    });

    it('turns :name: into the emoji in the middle of a line, keeping the text around it', () => {
      const t = new ComposerTyping(valueFromBlocks(['hello world'])).moveTo(0, 5);
      t.type(' :wave:');
      expect(t.blocks()).toEqual(['hello  world']);
      expect(t.inlines()).toEqual([{ type: 'emoji', data: { name: 'wave' } }]);
      t.type('!');
      expect(t.html()).toBe('<div>hello <span data-emoji-name="wave">👋</span>! world</div>');
    });
  });

  describe('Markdown shortcuts', () => {
    const cases: Array<[string, string, string]> = [
      ['**b**', 'b', '<div><strong>b</strong></div>'],
      ['**bold** text', 'bold text', '<div><strong>bold</strong>\u00a0text</div>'],
      ['**two words**,', 'two words,', '<div><strong>two words</strong>,</div>'],
      ['*i*', 'i', '<div><em>i</em></div>'],
      ['x *it* y', 'x it y', '<div>x <em>it</em>\u00a0y</div>'],
      ['`code`', 'code', '<div><code spellcheck="false">code</code></div>'],
      ['a `c` b', 'a c b', '<div>a <code spellcheck="false">c</code>\u00a0b</div>'],
      ['2*3*4', '2*3*4', '<div>2*3*4</div>'],
      ['a*b*c', 'a*b*c', '<div>a*b*c</div>'],
      ['5 * 3 * 2', '5 * 3 * 2', '<div>5 * 3 * 2</div>'],
      ['** not bold **', '** not bold **', '<div>** not bold **</div>'],
      ['snake_case_name', 'snake_case_name', '<div>snake_case_name</div>'],
      ['# not a heading', '# not a heading', '<div># not a heading</div>'],
      ['> q', '> q', '<div>&gt; q</div>'],
      ['-not a list', '-not a list', '<div>-not a list</div>'],
      ['2. two', '2. two', '<div>2. two</div>'],
    ];
    for (const [input, text, html] of cases) {
      it(`types ${JSON.stringify(input)} as ${JSON.stringify(text)}`, () => {
        const t = typed(input);
        expect(t.blocks()).toEqual([text]);
        expect(t.html()).toBe(html);
        expect(t.cursor().offset).toBe(text.length);
      });
    }

    it('stops the mark after the closing delimiter', () => {
      const t = typed('**b** c');
      expect(t.marksAt(0, 0)).toEqual(['bold']);
      expect(t.marksAt(0, 2)).toEqual([]);
    });

    it('starts lists with "- " and "1. "', () => {
      expect(typed('- item').html()).toBe('<ul><li><div>item</div></li></ul>');
      expect(typed('1. one').html()).toBe('<ol><li><div>one</div></li></ol>');
    });

    it('starts a code block with ```', () => {
      const t = typed('```x');
      expect(t.blockTypes()).toEqual(['code']);
      expect(t.blocks()).toEqual(['x']);
    });

    it('applies none of them inside a code block', () => {
      const text = 'a*b* **c** `d` - e 1. f ``` g';
      const t = new ComposerTyping(valueFromBlocks([{ type: 'code', text: '' }]));
      t.type(text);
      expect(t.blocks()).toEqual([text]);
      expect(t.blockTypes()).toEqual(['code']);
      t.type('{Enter}- x');
      expect(t.blocks()).toEqual([`${text}\n- x`]);
      expect(t.blockTypes()).toEqual(['code']);
    });

    it('starts no list inside a list item', () => {
      const t = new ComposerTyping(valueFromBlocks([list('one')]));
      t.type('{Enter}- two');
      expect(t.blocks()).toEqual(['one', '- two']);
      expect(t.blockTypes()).toEqual(['ul_list>list_item>div', 'ul_list>list_item>div']);
    });
  });

  describe('links', () => {
    it('links a URL on Space and keeps typing outside it', () => {
      const t = typed('see http://example.com/a?b=1 ok');
      expect(t.blocks()).toEqual(['see http://example.com/a?b=1 ok']);
      expect(t.html()).toBe(
        '<div>see <a href="http://example.com/a?b=1" title="http://example.com/a?b=1">http://example.com/a?b=1</a>\u00a0ok</div>'
      );
    });

    it('links a URL on Enter and still starts a new line', () => {
      const t = typed('http://example.com{Enter}next');
      expect(t.blocks()).toEqual(['http://example.com', 'next']);
      expect(t.html()).toBe(
        '<div><a href="http://example.com" title="http://example.com">http://example.com</a></div><div>next</div>'
      );
    });

    it('links a URL on Enter in a list and still adds a list item', () => {
      const t = new ComposerTyping(valueFromBlocks([list('one')]));
      t.type(' http://example.com{Enter}two');
      expect(t.blocks()).toEqual(['one http://example.com', 'two']);
      expect(t.blockTypes()).toEqual(['ul_list>list_item>div', 'ul_list>list_item>div']);
    });

    it('gives a link with no scheme an absolute href', () => {
      expect(typed('go to example.com now').html()).toBe(
        '<div>go to <a href="http://example.com" title="http://example.com">example.com</a>\u00a0now</div>'
      );
    });

    it('links an email address as mailto:', () => {
      expect(typed('mail a@b.co{Enter}x').html()).toBe(
        '<div>mail <a href="mailto:a@b.co" title="mailto:a@b.co">a@b.co</a></div><div>x</div>'
      );
    });

    it('links a pasted URL', () => {
      const t = new ComposerTyping(valueFromBlocks(['x ']));
      t.pastePlainText('http://example.com');
      expect(t.html()).toBe(
        '<div>x <a href="http://example.com" title="http://example.com">http://example.com</a></div>'
      );
    });

    it('links nothing in a code block', () => {
      const t = new ComposerTyping(valueFromBlocks([{ type: 'code', text: '' }]));
      t.type('http://example.com x');
      expect(t.html()).not.toContain('<a ');
    });
  });

  describe('lines, lists and quotes', () => {
    it('splits and joins lines with Enter and Backspace', () => {
      const t = typed('line1{Enter}line2');
      expect(t.blocks()).toEqual(['line1', 'line2']);
      t.type('{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}');
      expect(t.blocks()).toEqual(['line1']);
      expect(t.cursor()).toEqual({ block: 0, offset: 5, collapsed: true });
    });

    it('does nothing on Backspace at the start of the document or in an empty one', () => {
      expect(
        new ComposerTyping(valueFromBlocks(['abc'])).moveTo(0, 0).type('{Backspace}').blocks()
      ).toEqual(['abc']);
      expect(typed('{Backspace}{Backspace}x').blocks()).toEqual(['x']);
    });

    it('adds list items on Enter, ends the list on Enter in an empty one and nests on Tab', () => {
      const t = new ComposerTyping(valueFromBlocks([list('one')]));
      t.type('{Enter}two{Enter}{Tab}nested');
      expect(t.blocks()).toEqual(['one', 'two', 'nested']);
      expect(t.blockTypes()[2]).toBe('ul_list>list_item>ul_list>list_item>div');

      const out = new ComposerTyping(valueFromBlocks([list('one')]));
      out.type('{Enter}{Enter}after');
      expect(out.blocks()).toEqual(['one', 'after']);
      expect(out.blockTypes()).toEqual(['ul_list>list_item>div', 'div']);
    });

    it('breaks out of a quote on Enter, keeping the text on both sides', () => {
      const t = new ComposerTyping(
        valueFromBlocks(['top', { type: 'blockquote', nodes: ['abcdef'] }])
      ).moveTo(1, 3);
      t.type('{Enter}x');
      expect(t.blocks()).toEqual(['top', 'abc', 'xdef']);
      expect(t.blockTypes()).toEqual(['div', 'blockquote>div', 'div']);
    });

    it('types into a quote as anywhere else', () => {
      const t = new ComposerTyping(
        valueFromBlocks(['top', { type: 'blockquote', nodes: ['abc'] }])
      ).type(' :) **b** http://example.com');
      expect(t.blocks()).toEqual(['top', 'abc :) b http://example.com']);
    });
  });

  describe('other text', () => {
    it('types accented letters, CJK, emoji and RTL text as they are', () => {
      const text = 'héllo wörld ñ ç 日本語 한국어 😀👍🏽 שלום مرحبا';
      const t = typed(text);
      expect(t.blocks()).toEqual([text]);
      expect(t.cursor().offset).toBe(text.length);
    });

    it('deletes an astral emoji with one Backspace', () => {
      expect(typed('a😀{Backspace}b').blocks()).toEqual(['ab']);
    });

    it('inserts a committed IME composition once', () => {
      const t = new ComposerTyping(valueFromBlocks(['a'])).compose(['k', 'ko', 'kor'], 'これ');
      expect(t.blocks()).toEqual(['aこれ']);
      expect(t.cursor().offset).toBe(3);
    });

    it('leaves a composition after a colon alone', () => {
      const t = new ComposerTyping(valueFromBlocks(['a :'])).compose(['s', 'sm'], 'すみ');
      expect(t.blocks()).toEqual(['a :すみ']);
      expect(t.html()).toBe('<div>a :すみ</div>');
    });

    it('ignores Cmd and Ctrl shortcuts that type nothing', () => {
      const t = new ComposerTyping(valueFromBlocks(['abc']));
      t.press('a', { metaKey: true });
      t.press('c', { metaKey: true });
      t.press('c', { ctrlKey: true });
      expect(t.blocks()).toEqual(['abc']);
    });

    it('pastes CRLF text as lines with no stray carriage returns', () => {
      const t = new ComposerTyping();
      t.pastePlainText('a\r\n\r\nb');
      expect(t.blocks()).toEqual(['a', '', 'b']);
    });
  });

  describe('next to inline nodes', () => {
    it('types after an inline image that was just inserted', () => {
      const { changes } = require('../../src/components/composer-editor/inline-attachment-plugins');
      const t = new ComposerTyping(valueFromBlocks(['Hi ']));
      changes.insert(t.editor, { contentId: 'cid1' });
      t.flush();
      t.type('after{Enter}next');
      expect(t.blocks()).toEqual(['Hi after', 'next']);
      expect(t.html()).toBe('<div>Hi <img alt="" src="cid:cid1"/>after</div><div>next</div>');
    });

    it('types after an emoji whose node the cursor is in', () => {
      const t = new ComposerTyping(valueFromBlocks(['Hi ']));
      t.editor.insertInline({ object: 'inline', type: 'emoji', data: { name: 'smile' } } as any);
      t.flush();
      t.type('x');
      expect(t.blocks()).toEqual(['Hi x']);
      expect(t.html()).toBe('<div>Hi <span data-emoji-name="smile">😄</span>x</div>');
    });
  });

  describe('template variables', () => {
    it('leaves {{name}} as typed in a message', () => {
      const t = typed('Hi {{first}}, ok {{a} b}');
      expect(t.blocks()).toEqual(['Hi {{first}}, ok {{a} b}']);
      expect(t.inlines()).toEqual([]);
    });

    it('turns {{name}} into a variable in the template editor', () => {
      const t = typed('Hi {{first}}, ok', [''], { propsForPlugins: { inTemplateEditor: true } });
      expect(t.blocks()).toEqual(['Hi , ok']);
      expect(t.inlines()).toEqual([{ type: 'templatevar', data: { name: 'first' } }]);
    });

    it('replaces the variable at the cursor with what is typed, but not on a shortcut', () => {
      const t = new ComposerTyping(valueFromBlocks(['Hi ']));
      t.editor.insertInline({
        object: 'inline',
        type: 'templatevar',
        data: { name: 'first' },
      } as any);
      t.flush();
      t.press('c', { metaKey: true });
      expect(t.inlines().length).toBe(1);
      t.type('Ann');
      expect(t.inlines()).toEqual([]);
      expect(t.blocks()).toEqual(['Hi Ann']);
    });
  });
});

describe('Typing in the plaintext composer', () => {
  it('types every printable ASCII character as itself', () => {
    for (const ch of PRINTABLE_ASCII) {
      const p = new PlaintextTyping('').type(ch);
      expect(p.value).toBe(ch);
      expect(p.cursor()).toBe(1);
    }
    const all = PRINTABLE_ASCII.join('');
    expect(new PlaintextTyping('').type(all).value).toBe(all);
  });

  it('leaves Markdown, colons and URLs as typed', () => {
    const text = 'Hello: http://x.y **b** - item{Enter}1. one{Enter}> q :smile: {{x}}';
    const p = new PlaintextTyping('').type(text);
    expect(p.value).toBe('Hello: http://x.y **b** - item\n1. one\n> q :smile: {{x}}');
    expect(p.cursor()).toBe(p.value.length);
  });

  it('wraps at 72 columns with soft newlines, keeping the cursor after the last character', () => {
    const p = new PlaintextTyping('').type('word '.repeat(20));
    expect(p.value).toBe(`${'word '.repeat(13)}word\n${ZERO_WIDTH_SPACE}${'word '.repeat(6)}`);
    expect(p.cursor()).toBe(p.value.length);
  });

  it('keeps the cursor after a character typed inside a wrapped paragraph', () => {
    const p = new PlaintextTyping('').type('aaaa '.repeat(20).trim());
    p.el.setSelectionRange(10, 10);
    p.type('XYZ ');
    expect(p.value.slice(0, p.cursor())).toBe('aaaa aaaa XYZ ');
  });

  it('never puts a blank line before a word too long for a line', () => {
    const long = 'x'.repeat(100);
    expect(new PlaintextTyping('').type(long).value).toBe(long);
    expect(new PlaintextTyping('').type(` ${long}`).value).toBe(` ${long}`);
    expect(new PlaintextTyping('').type(`ab ${long} z`).value).toBe(
      `ab\n${ZERO_WIDTH_SPACE}${long}\n${ZERO_WIDTH_SPACE}z`
    );
  });

  it('keeps the quote prefix on a wrapped quoted line', () => {
    const p = new PlaintextTyping(`> ${'word '.repeat(13).trim()}`).type(' more more more');
    expect(p.value).toBe(`> ${'word '.repeat(13)}more\n${ZERO_WIDTH_SPACE}> more more`);
  });

  it('types accented letters, CJK and emoji as they are', () => {
    expect(new PlaintextTyping('').type('héllo 日本 😀 ñ').value).toBe('héllo 日本 😀 ñ');
  });
});
