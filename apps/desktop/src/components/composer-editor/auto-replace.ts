import AutoReplace, { AutoReplaceOptions } from 'slate-auto-replace';
import { Editor, Value } from 'slate';
import { ComposerEditorPlugin } from './types';

// Whether the cursor's block, or a block around it, is one of `types`.
export function isInBlockOfType(value: Value, types: string[]) {
  const block = value.startBlock;
  if (!block) return false;
  if (types.includes(block.type)) return true;
  const ancestors = value.document.getAncestors(block.key);
  return !!ancestors?.some((a) => a?.object === 'block' && types.includes(a.type));
}

// An AutoReplace that does nothing while the cursor is in one of the `outside` block types.
// slate-auto-replace 0.11 removed its own onlyIn / ignoreIn options and ignores them.
export function autoReplaceOutside(
  outside: string[],
  options: AutoReplaceOptions
): ComposerEditorPlugin {
  const plugin = AutoReplace(options);
  return {
    onKeyDown(event: React.KeyboardEvent, editor: Editor, next: () => void) {
      if (isInBlockOfType(editor.value, outside)) return next();
      return plugin.onKeyDown(event, editor, next);
    },
  };
}
