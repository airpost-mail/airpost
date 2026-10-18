/* eslint quote-props: 0 */
import ReactTestUtils from 'react-dom/test-utils';
import React from 'react';
import { SignatureStore, Actions } from 'mailspring-exports';
import PreferencesSignatures from '../lib/preferences-signatures';

const SIGNATURES = {
  '1': {
    id: '1',
    title: 'one',
    body: 'first test signature!',
  },
  '2': {
    id: '2',
    title: 'two',
    body: 'Here is my second sig!',
  },
};

const DEFAULTS = {
  'one@mailspring.com': '1',
  'two@mailspring.com': '2',
};

const makeComponent = (props = {}) => {
  return ReactTestUtils.renderIntoDocument(<PreferencesSignatures {...props} />);
};

describe('PreferencesSignatures', function preferencesSignatures() {
  this.component = null;

  describe('the preview', () => {
    it('renders the signature sanitized', () => {
      const sig = {
        id: '3',
        title: 'three',
        body: '<img src="x" onerror="window.__sigPreview = 1"><b>Ann</b><script>1</script>',
      };
      spyOn(SignatureStore, 'getSignatures').andReturn({ '3': sig });
      spyOn(SignatureStore, 'selectedSignature').andReturn(sig);
      spyOn(SignatureStore, 'getDefaults').andReturn({});
      const component = makeComponent() as any;
      const preview = ReactTestUtils.scryRenderedDOMComponentsWithClass(component, 'preview').find(
        (el) => el.tagName === 'DIV' && !el.classList.contains('section')
      );
      expect(preview.innerHTML).toContain('<b>Ann</b>');
      expect(preview.innerHTML).not.toContain('onerror');
      expect(preview.innerHTML).not.toContain('<script');
    });
  });

  describe('when there are signatures', () => {
    beforeEach(() => {
      spyOn(SignatureStore, 'getSignatures').andReturn(SIGNATURES);
      spyOn(SignatureStore, 'selectedSignature').andReturn(SIGNATURES['1']);
      spyOn(SignatureStore, 'getDefaults').andReturn(DEFAULTS);
      this.component = makeComponent();
    });
    it('should add a signature when you click the plus button', () => {
      spyOn(Actions, 'upsertSignature');
      this.plus = ReactTestUtils.scryRenderedDOMComponentsWithClass(
        this.component,
        'btn-editable-list'
      )[0];
      ReactTestUtils.Simulate.click(this.plus);
      expect(Actions.upsertSignature).toHaveBeenCalled();
    });
    it('should delete a signature when you click the minus button', () => {
      spyOn(Actions, 'removeSignature');
      this.minus = ReactTestUtils.scryRenderedDOMComponentsWithClass(
        this.component,
        'btn-editable-list'
      )[1];
      ReactTestUtils.Simulate.click(this.minus);
      expect(Actions.removeSignature).toHaveBeenCalledWith(SIGNATURES['1']);
    });
    it('should set the selected signature when you click on one that is not currently selected', () => {
      spyOn(Actions, 'selectSignature');
      this.item = ReactTestUtils.scryRenderedDOMComponentsWithClass(this.component, 'list-item')[1];
      ReactTestUtils.Simulate.click(this.item);
      expect(Actions.selectSignature).toHaveBeenCalledWith('2');
    });
    it('should modify the signature title when edited', () => {
      spyOn(Actions, 'upsertSignature');
      const newTitle = 'Changed';
      this.component._onEditSignatureTitle(newTitle);
      expect(Actions.upsertSignature).toHaveBeenCalled();
    });
  });

  describe('the preview', () => {
    const HOSTILE = {
      id: '3',
      title: 'hostile',
      body:
        '<b id="kept">Ann</b><img src="x" onerror="window.__sigXSS = true">' +
        '<script>window.__sigXSS = true</script><a href="javascript:alert(1)">link</a>',
    };

    beforeEach(() => {
      (window as any).__sigXSS = false;
      spyOn(SignatureStore, 'getSignatures').andReturn({ ...SIGNATURES, '3': HOSTILE });
      spyOn(SignatureStore, 'selectedSignature').andReturn(HOSTILE);
      spyOn(SignatureStore, 'getDefaults').andReturn(DEFAULTS);
      this.component = makeComponent();
    });

    it('renders the signature through SanitizeTransformer', () => {
      const preview = ReactTestUtils.scryRenderedDOMComponentsWithClass(
        this.component,
        'preview'
      ).find((el) => el.tagName === 'DIV' && el.parentElement.classList.contains('section'));
      expect(preview.querySelector('#kept').textContent).toBe('Ann');
      expect(preview.innerHTML).not.toContain('onerror');
      expect(preview.innerHTML).not.toContain('<script');
      expect(preview.innerHTML).not.toContain('javascript:');
      expect((window as any).__sigXSS).toBe(false);
    });
  });
});
