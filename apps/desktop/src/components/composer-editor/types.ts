import { Node, Editor, Value, Mark, Block, Inline, Plugin as SlateCorePlugin } from 'slate';
import { Plugin, RenderAttributes, RenderMarkProps } from 'slate-react';
import { DraftEditingSession, MessageWithEditorState } from '../../data/drafts/editing-session';

export interface Rule {
  deserialize?: (
    el: Element,
    next: (elements: Element[] | NodeList | Array<Node & ChildNode>) => any
  ) => any;
  serialize?: (obj: any, children: string) => React.ReactNode | void;
}

export interface ComposerEditorPlugin
  extends Omit<Plugin<Editor>, 'onKeyDown'>, Omit<SlateCorePlugin, 'onChange'> {
  renderMark?: (
    { mark, children, targetIsHTML }: RenderMarkProps & { targetIsHTML?: boolean },
    editor?: Editor,
    next?: () => void
  ) => void | string | JSX.Element;

  renderNode?: (
    props: ComposerRenderNodeProps,
    editor: Editor | null,
    next: () => void
  ) => void | JSX.Element;

  rules?: Rule[];

  topLevelComponent?: React.ComponentType<ComposerEditorPluginTopLevelComponentProps>;
  toolbarComponents?: React.ComponentType<ComposerEditorPluginToolbarComponentProps>[];
  toolbarSectionClass?: string;

  appCommands?: { [command: string]: (event: CustomEvent, editor: Editor) => Editor | void };

  onChange?(editor: Editor, next: () => void): void;
  onKeyDown?: (event: React.KeyboardEvent, editor: Editor, next: () => void) => void;
  onKeyUp?: (event: React.KeyboardEvent, editor: Editor, next: () => void) => void;
}

// What ComposerEditor passes to slate-react as `propsForPlugins`.
export interface ComposerPropsForPlugins {
  draft?: MessageWithEditorState;
  session?: DraftEditingSession;
  inTemplateEditor?: boolean;
}

// The editor slate-react hands to render functions: its Editor component, which proxies
// the core Editor and carries ComposerEditor's props.
export type ComposerPluginEditor = Editor & { props: { propsForPlugins: ComposerPropsForPlugins } };

export interface ComposerRenderNodeProps {
  node: Block | Inline;
  children: any;
  targetIsHTML: boolean;
  // Passed by slate-react when rendering in the editor; absent when serializing to HTML.
  attributes?: RenderAttributes;
  editor?: ComposerPluginEditor;
  isFocused?: boolean;
  isSelected?: boolean;
}

export interface ComposerEditorPluginToolbarComponentProps {
  editor: Editor;
  value: Value;
  // The plugin's toolbarSectionClass, if it has one
  className?: string;
}

export interface ComposerEditorPluginTopLevelComponentProps {
  editor: Editor;
  value: Value;
}
