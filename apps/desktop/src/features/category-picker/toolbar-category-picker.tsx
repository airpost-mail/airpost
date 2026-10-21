import React, { useContext, useMemo, useRef, useCallback } from 'react';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { AccountStore } from '../../data/stores/account';
import WorkspaceStore from '../../data/stores/workspace';
import { SheetDepthContext } from '../../components/sheets/context';
import { Thread } from '../../data/models/thread';
import { KeyCommandsRegion } from '../../components/key-commands-region';
import { RovingTabIndexToolbar } from '../../components/roving-tab-index-toolbar';
import { IconButton } from '../../components/kit/icon-button';

import MovePickerPopover from './move-picker-popover';
import LabelPickerPopover from './label-picker-popover';

// This sets the folder / label on one or more threads.
const MovePicker: React.FC<{ items: Thread[] }> & { containerRequired?: boolean } = ({ items }) => {
  const sheetDepth = useContext(SheetDepthContext);
  const moveEl = useRef<HTMLButtonElement>(null);
  const labelEl = useRef<HTMLButtonElement>(null);

  // If the threads we're picking categories for change, (like when they
  // get their categories updated), we expect our parents to pass us new
  // props. We don't listen to the DatabaseStore ourselves.
  const account = useMemo(() => AccountStore.accountForItems(items), [items]);

  const onOpenMovePopover = useCallback(() => {
    if (items.length === 0) return;
    if (sheetDepth !== WorkspaceStore.sheetStack().length - 1) return;
    Actions.openPopover(<MovePickerPopover threads={items} account={account} />, {
      originRect: moveEl.current.getBoundingClientRect(),
      direction: 'down',
    });
  }, [items, account, sheetDepth]);

  const onOpenLabelsPopover = useCallback(() => {
    if (items.length === 0) return;
    if (sheetDepth !== WorkspaceStore.sheetStack().length - 1) return;
    Actions.openPopover(<LabelPickerPopover threads={items} account={account} />, {
      originRect: labelEl.current.getBoundingClientRect(),
      direction: 'down',
    });
  }, [items, account, sheetDepth]);

  if (!account) {
    return <span />;
  }

  const handlers: Record<string, () => void> = {
    'core:change-folders': onOpenMovePopover,
  };
  if (account.usesLabels()) {
    handlers['core:change-labels'] = onOpenLabelsPopover;
  }

  return (
    <RovingTabIndexToolbar
      label={localized('Category Actions')}
      className="button-group"
      style={{ order: -103 }}
    >
      <KeyCommandsRegion globalHandlers={handlers}>
        <IconButton
          variant="toolbar"
          tabIndex={-1}
          ref={moveEl}
          label={localized('Move to Folder')}
          icon="toolbar-movetofolder.png"
          onClick={onOpenMovePopover}
          className="btn-category-picker"
        />
        {account.usesLabels() && (
          <IconButton
            variant="toolbar"
            tabIndex={-1}
            ref={labelEl}
            label={localized('Apply Label')}
            icon="toolbar-tag.png"
            onClick={onOpenLabelsPopover}
            className="btn-category-picker"
          />
        )}
      </KeyCommandsRegion>
    </RovingTabIndexToolbar>
  );
};
MovePicker.displayName = 'MovePicker';
MovePicker.containerRequired = false;

export default MovePicker;
