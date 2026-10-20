import _ from 'underscore';
import React from 'react';
import * as Actions from '../data/actions';
import { localized } from '../shared/intl';
import { Dialog, DialogContent } from './kit/dialog';

type ModalProps = {
  className?: string;
  height?: number;
  width?: number;
  /** Read by screen readers; the modal shows no title of its own. */
  title?: string;
  children?: React.ReactNode;
};

/* The element that took focus when a modal opened before the kit: the lowest positive
   tabIndex, then the first input, then any other element with a tabIndex. */
function legacyFocusTarget(root: HTMLElement): HTMLElement | null {
  const focusable = root.querySelectorAll<HTMLElement>('[tabIndex], input');
  const matches = _.sortBy(Array.from(focusable), (node) => {
    if (node.tabIndex > 0) return node.tabIndex;
    if (node.nodeName === 'INPUT') return 1000000;
    return 1000001;
  });
  return matches[0] || null;
}

/* Public: The window's modal, opened with `Actions.openModal({ component, width, height })` and
   closed with `Actions.closeModal()`, by clicking outside it, with Escape or with its close
   button. The UI kit's Dialog behind this export's props, with the legacy `modal-container`,
   `modal` and `modal-close` classes. */
class Modal extends React.Component<ModalProps> {
  static displayName = 'Modal';

  _onOpenAutoFocus = (event: Event) => {
    const target = legacyFocusTarget(event.currentTarget as HTMLElement);
    if (target) {
      event.preventDefault();
      target.focus();
    }
  };

  render() {
    const { children, height, width, className, title } = this.props;
    return (
      <Dialog open onOpenChange={(open) => !open && Actions.closeModal()}>
        <DialogContent
          className={className}
          hiddenTitle={title || localized('Dialog')}
          aria-describedby={undefined}
          onOpenAutoFocus={this._onOpenAutoFocus}
          // The legacy modal's box: the size asked for, no padding of its own.
          style={{
            width,
            height,
            maxWidth: '95%',
            maxHeight: '95%',
            padding: 0,
            gap: 0,
            display: 'block',
          }}
        >
          {children}
        </DialogContent>
      </Dialog>
    );
  }
}

export default Modal;
