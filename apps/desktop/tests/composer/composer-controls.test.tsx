// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Actions } from 'mailspring-exports';
import ComposeButton from '../../src/features/composer/compose-button';
import ComposerHeaderActions from '../../src/features/composer/header-actions';
import { QuotedTextControl } from '../../src/features/composer/quoted-text-control';
import Fields from '../../src/features/composer/fields';

describe('Composer controls on the kit', function () {
  afterEach(() => cleanup());

  describe('ComposeButton', () => {
    it('is a toolbar icon button that keeps the item-compose class', () => {
      spyOn(Actions, 'composeNewBlankDraft');
      const { container } = render(<ComposeButton />);
      const button = container.querySelector('button');
      expect(button.getAttribute('data-slot')).toBe('icon-button');
      expect(button.getAttribute('data-variant')).toBe('toolbar');
      expect(button.className).toContain('btn btn-toolbar');
      expect(button.classList.contains('item-compose')).toBe(true);
      expect(button.getAttribute('aria-label')).toBe('Compose new message');
      expect(button.hasAttribute('title')).toBe(false);
      fireEvent.click(button);
      expect(Actions.composeNewBlankDraft).toHaveBeenCalled();
    });
  });

  describe('ComposerHeaderActions', () => {
    it('renders the field toggles as kit link buttons and popout as an icon button', () => {
      const { container } = render(
        <ComposerHeaderActions
          headerMessageId="a"
          enabledFields={[Fields.To]}
          onShowAndFocusField={() => {}}
        />
      );
      const toggles = ['show-cc', 'show-bcc', 'show-reply-to', 'show-subject'].map((c) =>
        container.querySelector(`.${c}`)
      );
      for (const el of toggles) {
        expect(el.tagName).toBe('BUTTON');
        expect(el.getAttribute('data-slot')).toBe('button');
        expect(el.getAttribute('data-variant')).toBe('link');
        expect(el.classList.contains('action')).toBe(true);
      }
      expect(toggles.map((el) => el.textContent)).toEqual(['Cc', 'Bcc', 'Reply-To', 'Subject']);
      const popout = container.querySelector('.show-popout');
      expect(popout.getAttribute('data-slot')).toBe('icon-button');
      expect(popout.getAttribute('aria-label')).toBe('Popout composer…');
      expect(popout.hasAttribute('title')).toBe(false);
    });
  });

  describe('QuotedTextControl', () => {
    const renderControl = () => {
      const onUnhide = jasmine.createSpy('onUnhide');
      const onRemove = jasmine.createSpy('onRemove');
      // Stands in for the composer body, whose mouseup handler moves the caret.
      const onBodyMouseUp = jasmine.createSpy('onBodyMouseUp');
      const { container } = render(
        <div onMouseUp={onBodyMouseUp}>
          <QuotedTextControl
            quotedTextPresent
            quotedTextHidden
            onUnhide={onUnhide}
            onRemove={onRemove}
          />
        </div>
      );
      return { container, onUnhide, onRemove, onBodyMouseUp };
    };

    it('renders nothing unless quoted text is present and hidden', () => {
      const { container } = render(
        <QuotedTextControl
          quotedTextPresent
          quotedTextHidden={false}
          onUnhide={() => {}}
          onRemove={() => {}}
        />
      );
      expect(container.innerHTML).toBe('');
    });

    it('shows the quoted text on a mousedown on the dots', () => {
      const { container, onUnhide, onRemove } = renderControl();
      fireEvent.mouseDown(container.querySelector('.dots'));
      expect(onUnhide).toHaveBeenCalled();
      expect(onRemove).not.toHaveBeenCalled();
    });

    it('removes the quoted text from a kit icon button on mouseup, without unhiding it first', () => {
      const { container, onUnhide, onRemove, onBodyMouseUp } = renderControl();
      const remove = container.querySelector('.remove-quoted-text');
      expect(remove.getAttribute('data-slot')).toBe('icon-button');
      expect(remove.getAttribute('aria-label')).toBe('Remove quoted text');
      expect(remove.getAttribute('tabindex')).toBe('-1');
      const down = fireEvent.mouseDown(remove);
      // The mousedown is cancelled, so focus stays where it was.
      expect(down).toBe(false);
      expect(onUnhide).not.toHaveBeenCalled();
      fireEvent.mouseUp(remove);
      expect(onRemove).toHaveBeenCalled();
      expect(onBodyMouseUp).not.toHaveBeenCalled();
    });
  });
});
