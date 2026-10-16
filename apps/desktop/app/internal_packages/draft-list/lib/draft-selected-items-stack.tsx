import React from 'react';
import { localized } from 'mailspring-exports';
import DraftListStore from './draft-list-store';

const MAX_CARDS = 5;

// Same markup as the thread list's SelectedItemsStack so its global styles apply.
export default function DraftSelectedItemsStack({ selectionCount }: { selectionCount: number }) {
  const cardCount = Math.min(MAX_CARDS, selectionCount);
  const cards = [];
  for (let idx = 0; idx < cardCount; idx++) {
    let transform = `rotate(${idx * 0.9 + (idx === 1 ? 0.5 : 0)}deg)`;
    if (idx === cardCount - 1) {
      transform += ' translate3d(2px, 3px, 0)';
    }
    cards.push(
      <div key={`card-${idx}`} style={{ transform, zIndex: MAX_CARDS - idx }} className="card">
        <svg width="100%" height="100%" viewBox="0 0 390 527" version="1.1">
          <rect strokeWidth="2.5" x="0" y="0" width="390" height="527" rx="15" />
        </svg>
      </div>
    );
  }

  return (
    <div className="selected-items-stack draft-selected-items-stack">
      <div className="selected-items-stack-content">
        <div className="stack">{cards}</div>
        <div className="count-info">
          <div className="count">{selectionCount}</div>
          <div className="count-message">{localized('Selected Drafts')}</div>
          <div className="clear btn" onClick={() => DraftListStore.dataSource().selection.clear()}>
            {localized('Clear Selection')}
          </div>
        </div>
      </div>
    </div>
  );
}
DraftSelectedItemsStack.displayName = 'DraftSelectedItemsStack';
