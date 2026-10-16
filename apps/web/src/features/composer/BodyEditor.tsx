// The message body: a contentEditable with execCommand formatting. No editor library is
// installed; execCommand is deprecated but still the one API every browser ships for
// bold/italic/lists/links with native undo.
import { useImperativeHandle, useLayoutEffect, useRef, useState, type Ref } from 'react';
import { cn } from '@/lib/utils';
import { escapeHtml, isBlankHtml, sanitizeEditorHtml, textToHtml } from './html';

export type FormatCommand =
  | 'bold'
  | 'italic'
  | 'underline'
  | 'strikeThrough'
  | 'insertUnorderedList'
  | 'insertOrderedList'
  | 'formatBlock'
  | 'removeFormat'
  | 'unlink';

export interface BodyEditorHandle {
  focus(): void;
  exec(command: FormatCommand, value?: string): void;
  /** Remembers the caret so a popover can take focus and give it back. */
  saveSelection(): void;
  insertLink(url: string): void;
  insertHtml(html: string): void;
  element(): HTMLDivElement | null;
}

export interface BodyEditorProps {
  initialHtml: string;
  onChange: (html: string) => void;
  onFiles?: (files: File[]) => void;
  onLinkRequest?: () => void;
  placeholder?: string;
  className?: string;
  ref?: Ref<BodyEditorHandle>;
}

function exec(command: string, value?: string): boolean {
  // jsdom and very old engines lack it; the editor still works as plain contentEditable.
  if (typeof document.execCommand !== 'function') return false;
  return document.execCommand(command, false, value);
}

export function BodyEditor({
  initialHtml,
  onChange,
  onFiles,
  onLinkRequest,
  placeholder = 'Write your message',
  className,
  ref,
}: BodyEditorProps) {
  const el = useRef<HTMLDivElement>(null);
  const saved = useRef<Range | null>(null);
  const [empty, setEmpty] = useState(() => isBlankHtml(initialHtml));
  const initial = useRef(initialHtml);

  useLayoutEffect(() => {
    if (el.current) el.current.innerHTML = initial.current;
  }, []);

  const emit = () => {
    const html = el.current?.innerHTML ?? '';
    setEmpty(isBlankHtml(html));
    onChange(html);
  };

  const restore = () => {
    el.current?.focus();
    const range = saved.current;
    if (!range) return;
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
    saved.current = null;
  };

  const insertHtml = (html: string) => {
    if (!exec('insertHTML', html) && el.current) {
      el.current.insertAdjacentHTML('beforeend', html);
    }
    emit();
  };

  useImperativeHandle(ref, () => ({
    focus: () => el.current?.focus(),
    exec(command, value) {
      el.current?.focus();
      exec(command, value);
      emit();
    },
    saveSelection() {
      const sel = window.getSelection();
      if (sel && sel.rangeCount && el.current?.contains(sel.anchorNode)) {
        saved.current = sel.getRangeAt(0).cloneRange();
      }
    },
    insertLink(url) {
      restore();
      const sel = window.getSelection();
      if (sel && !sel.isCollapsed) exec('createLink', url);
      else insertHtml(`<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`);
      emit();
    },
    insertHtml(html) {
      restore();
      insertHtml(sanitizeEditorHtml(html));
    },
    element: () => el.current,
  }));

  const onPaste = (e: React.ClipboardEvent<HTMLDivElement>) => {
    const data = e.clipboardData;
    const html = data.getData('text/html');
    const text = data.getData('text/plain');
    if (!html && !text && data.files.length) {
      e.preventDefault();
      onFiles?.([...data.files]);
      return;
    }
    e.preventDefault();
    insertHtml(html ? sanitizeEditorHtml(html) : textToHtml(text));
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod || e.altKey) return;
    const key = e.key.toLowerCase();
    if (key === 'k' && !e.shiftKey) {
      // Before the global palette shortcut sees it.
      e.preventDefault();
      const sel = window.getSelection();
      if (sel?.rangeCount) saved.current = sel.getRangeAt(0).cloneRange();
      onLinkRequest?.();
    } else if (e.shiftKey && (e.code === 'Digit7' || e.code === 'Digit8')) {
      e.preventDefault();
      exec(e.code === 'Digit7' ? 'insertOrderedList' : 'insertUnorderedList');
      emit();
    }
  };

  return (
    <div
      ref={el}
      role="textbox"
      aria-multiline
      aria-label="Message body"
      contentEditable
      suppressContentEditableWarning
      data-empty={empty || undefined}
      data-placeholder={placeholder}
      onInput={emit}
      onPaste={onPaste}
      onKeyDown={onKeyDown}
      className={cn(
        'relative min-h-32 px-(--pane-gutter) py-3 text-base break-words outline-none',
        'data-empty:before:pointer-events-none data-empty:before:absolute data-empty:before:text-muted-foreground data-empty:before:content-[attr(data-placeholder)]',
        '[&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2',
        '[&_ol]:list-decimal [&_ol]:pl-6 [&_ul]:list-disc [&_ul]:pl-6',
        '[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground',
        '[&_pre]:rounded-sm [&_pre]:bg-muted [&_pre]:p-2 [&_pre]:font-mono [&_pre]:text-sm',
        className,
      )}
    />
  );
}
