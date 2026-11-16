// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// Types for the untyped Slate plugin packages the composer uses, as far as it uses them.

declare module 'slate-soft-break' {
  interface SoftBreakPlugin {
    onKeyDown: (
      event: import('react').KeyboardEvent,
      editor: import('slate').Editor,
      next: () => void
    ) => unknown;
  }
  export default function SoftBreak(options?: { shift?: boolean }): SoftBreakPlugin;
}

declare module 'slate-when' {
  type When = <P extends object>(options: {
    when: (value: import('slate').Value) => boolean;
    plugin: P;
  }) => Partial<P>;
  const when: When;
  export default when;
}

declare module 'slate-auto-replace' {
  export interface AutoReplaceMatches {
    before: RegExpMatchArray | null;
    after: RegExpMatchArray | null;
  }
  export interface AutoReplaceOptions {
    trigger: string | RegExp | ((event: import('react').KeyboardEvent) => boolean);
    before?: RegExp;
    after?: RegExp;
    change: (
      editor: import('slate').Editor,
      event: import('react').KeyboardEvent,
      matches: AutoReplaceMatches
    ) => unknown;
  }
  interface AutoReplacePlugin {
    onKeyDown: (
      event: import('react').KeyboardEvent,
      editor: import('slate').Editor,
      next: () => void
    ) => unknown;
  }
  export default function AutoReplace(options: AutoReplaceOptions): AutoReplacePlugin;
}

declare module '@bengotow/slate-edit-list' {
  import { Block, Editor, Value } from 'slate';

  interface EditListOptions {
    types?: string[];
    typeItem?: string;
    typeDefault?: string;
  }
  interface EditListPlugin {
    schema: import('slate').SchemaProperties;
    normalizeNode: (
      node: import('slate').Node,
      editor: Editor,
      next: () => void
    ) => ((editor: Editor) => void) | void;
    onKeyDown: (event: import('react').KeyboardEvent, editor: Editor, next: () => void) => unknown;
    utils: {
      getCurrentItem(value: Value, block?: Block): Block | null;
      getCurrentList(value: Value, block?: Block): Block | null;
      getItemDepth(value: Value, block?: Block): number;
      isList(node: import('slate').Node): boolean;
      isSelectionInList(value: Value): boolean;
    };
    changes: {
      decreaseItemDepth(editor: Editor): Editor;
      increaseItemDepth(editor: Editor): Editor;
      splitListItem(editor: Editor): Editor;
      unwrapList(editor: Editor): Editor;
      wrapInList(editor: Editor, type?: string, data?: object): Editor;
      toggleList(editor: Editor, type?: string, data?: object): Editor;
    };
  }
  // A factory that is also called with `new`; it returns the plugin object either way.
  const EditList: {
    new (options?: EditListOptions): EditListPlugin;
    (options?: EditListOptions): EditListPlugin;
  };
  export default EditList;
}

declare module 'react-color/lib/Compact' {
  export { default } from 'react-color/lib/components/compact/Compact';
}
