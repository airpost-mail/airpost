import React from 'react';
import { localized } from '../../intl/localize';
import { RetinaImg } from '../../components/retina-img';
import { IconButton } from '../../components/kit/icon-button';

export const QuotedTextControl: React.FunctionComponent<{
  quotedTextPresent: boolean;
  quotedTextHidden: boolean;
  onUnhide: () => void;
  onRemove: () => void;
}> = (props) => {
  if (!props.quotedTextPresent || !props.quotedTextHidden) {
    return null;
  }
  return (
    <a
      className="quoted-text-control"
      onMouseDown={(e) => {
        if (e.target instanceof HTMLElement && e.target.closest('.remove-quoted-text')) return;
        e.preventDefault();
        e.stopPropagation();
        props.onUnhide();
      }}
    >
      <span className="dots">&bull;&bull;&bull;</span>
      {/* Shown only on hover, so it isn't a tab stop. Mousedown doesn't take focus, so focus
      stays in the composer when the control goes away. It acts on mouseup and stops it there,
      so the composer body's mouseup handler doesn't move the caret to the end of the draft. */}
      <IconButton
        className="remove-quoted-text"
        tabIndex={-1}
        icon="image-cancel-button.png"
        mode={RetinaImg.Mode.ContentPreserve}
        label={localized('Remove quoted text')}
        onMouseDown={(e) => e.preventDefault()}
        onMouseUp={(e) => {
          e.preventDefault();
          e.stopPropagation();
          props.onRemove();
        }}
      />
    </a>
  );
};
