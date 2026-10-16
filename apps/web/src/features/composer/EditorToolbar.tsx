import {
  Bold,
  Italic,
  Link2,
  List,
  ListOrdered,
  RemoveFormatting,
  Strikethrough,
  TextQuote,
  Underline,
  type LucideIcon,
} from 'lucide-react';
import { useEffect, useState, type RefObject } from 'react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type { BodyEditorHandle, FormatCommand } from './BodyEditor';
import { normalizeUrl } from './html';

interface Tool {
  command: FormatCommand;
  value?: string;
  label: string;
  icon: LucideIcon;
  keys?: string;
  /** queryCommandState name, when the button shows an on/off state. */
  state?: string;
}

const TOOLS: Tool[] = [
  { command: 'bold', label: 'Bold', icon: Bold, keys: 'mod+b', state: 'bold' },
  { command: 'italic', label: 'Italic', icon: Italic, keys: 'mod+i', state: 'italic' },
  { command: 'underline', label: 'Underline', icon: Underline, keys: 'mod+u', state: 'underline' },
  { command: 'strikeThrough', label: 'Strikethrough', icon: Strikethrough, state: 'strikeThrough' },
  {
    command: 'insertUnorderedList',
    label: 'Bulleted list',
    icon: List,
    keys: 'mod+shift+8',
    state: 'insertUnorderedList',
  },
  {
    command: 'insertOrderedList',
    label: 'Numbered list',
    icon: ListOrdered,
    keys: 'mod+shift+7',
    state: 'insertOrderedList',
  },
  { command: 'formatBlock', value: 'blockquote', label: 'Quote', icon: TextQuote },
  { command: 'removeFormat', label: 'Clear formatting', icon: RemoveFormatting },
];

function useFormatState(editor: RefObject<BodyEditorHandle | null>) {
  const [active, setActive] = useState<Record<string, boolean>>({});
  useEffect(() => {
    if (typeof document.queryCommandState !== 'function') return;
    const update = () => {
      const root = editor.current?.element();
      const sel = window.getSelection();
      if (!root || !sel?.anchorNode || !root.contains(sel.anchorNode)) return;
      const next: Record<string, boolean> = {};
      for (const t of TOOLS) if (t.state) next[t.state] = document.queryCommandState(t.state);
      setActive((prev) =>
        TOOLS.every((t) => !t.state || prev[t.state] === next[t.state]) ? prev : next,
      );
    };
    document.addEventListener('selectionchange', update);
    return () => document.removeEventListener('selectionchange', update);
  }, [editor]);
  return active;
}

function ToolButton({
  label,
  keys,
  pressed,
  onPress,
  children,
}: {
  label: string;
  keys?: string;
  pressed?: boolean;
  onPress: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          size="icon-xs"
          variant="ghost"
          aria-label={label}
          aria-pressed={pressed}
          // Keep the editor's selection; the command applies to it.
          onMouseDown={(e) => e.preventDefault()}
          onClick={onPress}
          className="text-muted-foreground hover:text-foreground aria-pressed:bg-accent aria-pressed:text-foreground"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent className="flex items-center gap-2">
        {label}
        {keys && <ShortcutKeys keys={keys} />}
      </TooltipContent>
    </Tooltip>
  );
}

export function EditorToolbar({
  editor,
  linkOpen,
  onLinkOpenChange,
}: {
  editor: RefObject<BodyEditorHandle | null>;
  linkOpen: boolean;
  onLinkOpenChange: (open: boolean) => void;
}) {
  const active = useFormatState(editor);

  const openLink = () => {
    editor.current?.saveSelection();
    onLinkOpenChange(true);
  };

  return (
    <div role="toolbar" aria-label="Formatting" className="flex items-center gap-px">
      {TOOLS.slice(0, 4).map((t) => (
        <ToolButton
          key={t.command}
          label={t.label}
          keys={t.keys}
          pressed={t.state ? !!active[t.state] : undefined}
          onPress={() => editor.current?.exec(t.command, t.value)}
        >
          <t.icon aria-hidden />
        </ToolButton>
      ))}
      <Popover open={linkOpen} onOpenChange={onLinkOpenChange}>
        <PopoverAnchor asChild>
          <span>
            <ToolButton label="Link" keys="mod+k" onPress={openLink}>
              <Link2 aria-hidden />
            </ToolButton>
          </span>
        </PopoverAnchor>
        <PopoverContent
          side="top"
          align="start"
          className="w-72 p-2"
          onCloseAutoFocus={(e) => {
            e.preventDefault();
            editor.current?.focus();
          }}
        >
          <LinkForm
            onSubmit={(url) => {
              onLinkOpenChange(false);
              editor.current?.insertLink(url);
            }}
          />
        </PopoverContent>
      </Popover>
      {TOOLS.slice(4).map((t) => (
        <ToolButton
          key={t.command + (t.value ?? '')}
          label={t.label}
          keys={t.keys}
          pressed={t.state ? !!active[t.state] : undefined}
          onPress={() => editor.current?.exec(t.command, t.value)}
        >
          <t.icon aria-hidden />
        </ToolButton>
      ))}
    </div>
  );
}

function LinkForm({ onSubmit }: { onSubmit: (url: string) => void }) {
  const [url, setUrl] = useState('');
  const valid = normalizeUrl(url);
  return (
    <form
      className="flex gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (valid) onSubmit(valid);
      }}
    >
      <Input
        autoFocus
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        placeholder="Paste or type a link"
        aria-label="Link address"
        className="h-7 text-sm"
      />
      <Button type="submit" size="sm" disabled={!valid}>
        Add
      </Button>
    </form>
  );
}
