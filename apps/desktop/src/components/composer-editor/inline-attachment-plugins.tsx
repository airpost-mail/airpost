import React from 'react';
import { ImageAttachmentItem } from '../attachment-items';
import AttachmentStore from '../../data/stores/attachment';
import { File as FileModel } from '../../data/models/file';
import { isQuoteNode } from './base-block-plugins';
import { ComposerEditorPlugin, ComposerRenderNodeProps, Rule } from './types';
import { Editor, Inline, Node } from 'slate';
import { schema } from './conversion';

export const IMAGE_TYPE = 'image';

function ImageNode(props: ComposerRenderNodeProps) {
  const { attributes, node, editor, targetIsHTML, isFocused } = props;
  const contentId = node.data.get('contentId'),
    imgProps = node.data.get('imgProps');

  if (targetIsHTML) {
    return (
      <img alt="" src={`cid:${contentId}`} width={imgProps?.width} height={imgProps?.height} />
    );
  }

  // slate-react always passes the editor when rendering into it
  const { draft } = editor!.props.propsForPlugins;
  // The template editor has no draft
  const file = draft?.files.find((f) => contentId === f.contentId);
  if (!file) {
    return <span {...attributes} />;
  }

  return (
    <ImageAttachmentItem
      {...attributes}
      className={`file-upload custom-block ${isFocused && 'focused'}`}
      // pathForFile only returns null for a missing file
      filePath={AttachmentStore.pathForFile(file)!}
      displayName={file.filename}
      onRemoveAttachment={() => editor!.removeNodeByKey(node.key)}
      imgProps={imgProps}
      onResized={(width, height) => {
        const e = editor as Editor,
          n = node as Inline,
          newN = {
            key: n.key,
            object: n.object,
            data: n.data.asMutable(),
            type: n.type,
            nodes: n.nodes.asMutable(),
          };

        newN.data = newN.data.set('imgProps', { width: width, height: height });

        e.setNodeByKey(n.key, newN);
      }}
    />
  );
}

function renderNode(
  props: ComposerRenderNodeProps,
  editor: Editor | null = null,
  next: () => void = () => {}
) {
  if (props.node.type === IMAGE_TYPE) {
    return ImageNode(props);
  }
  return next();
}

const rules: Rule[] = [
  {
    deserialize(el, next) {
      if (el.tagName.toLowerCase() !== 'img') return;
      const src = el.getAttribute('src') || '';
      if (src.startsWith('cid:')) {
        return {
          object: 'inline',
          nodes: [],
          type: IMAGE_TYPE,
          data: {
            contentId: src.split('cid:').pop(),
            imgProps: {
              // A missing attribute gives NaN, as parseInt(null) did
              width: Number.parseInt(el.getAttribute('width') ?? ''),
              height: Number.parseInt(el.getAttribute('height') ?? ''),
            },
          },
        };
      }
    },
    serialize(obj: any, children: any) {
      if (obj.object !== 'inline') return;
      return renderNode({ node: obj, children, targetIsHTML: true });
    },
  },
];

export const changes = {
  insert: (editor: Editor, file: FileModel) => {
    const canHoldInline = (node: Node) => {
      const isVoid =
        node.object === 'inline' && schema.inlines[node.type] && schema.inlines[node.type].isVoid;
      return !isVoid && !isQuoteNode(node) && !!node.getFirstText();
    };

    while (!canHoldInline(editor.value.anchorBlock)) {
      editor.moveToEndOfPreviousText();
      if (!editor.value.anchorBlock) {
        break;
      }
    }
    // insertInline leaves the cursor inside the image, where Enter only steps past it.
    return editor
      .insertInline({
        object: 'inline',
        type: IMAGE_TYPE,
        data: {
          contentId: file.contentId,
        },
      })
      .moveToStartOfNextText();
  },
};

const plugins: ComposerEditorPlugin[] = [
  {
    renderNode,
    rules,
  },
];

export default plugins;
