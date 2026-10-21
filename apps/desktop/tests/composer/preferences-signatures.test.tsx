/* eslint quote-props: 0 */
import ReactTestUtils from 'react-dom/test-utils';
import React from 'react';
import ReactDOM from 'react-dom';
import { SignatureStore } from '../../src/data/stores/signature';
import * as Actions from '../../src/data/actions';
import PreferencesSignatures from '../../src/features/composer-signature/preferences-signatures';
import Templates from '../../src/features/composer-signature/templates';
import { Dialogs } from '../../src/platform/app-api';

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
  'one@example.com': '1',
  'two@example.com': '2',
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
      const preview = ReactTestUtils.findRenderedDOMComponentWithClass(
        component,
        'signature-preview'
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
    it('should add a signature when you click New signature', () => {
      spyOn(Actions, 'upsertSignature');
      spyOn(Actions, 'selectSignature');
      const add = ReactTestUtils.findRenderedDOMComponentWithClass(
        this.component,
        'settings-split-new'
      );
      ReactTestUtils.Simulate.click(add);
      expect(Actions.upsertSignature).toHaveBeenCalled();
      const id = (Actions.upsertSignature as unknown as jasmine.Spy).mostRecentCall.args[1];
      expect(Actions.selectSignature).toHaveBeenCalledWith(id);
    });
    it('should delete the selected signature when you click Delete', () => {
      spyOn(Actions, 'removeSignature');
      const del = ReactTestUtils.findRenderedDOMComponentWithClass(this.component, 'btn-danger');
      ReactTestUtils.Simulate.click(del);
      expect(Actions.removeSignature).toHaveBeenCalledWith(SIGNATURES['1']);
    });
    it('should mark the selected signature in the list', () => {
      const items = ReactTestUtils.scryRenderedDOMComponentsWithClass(this.component, 'list-item');
      expect(items.map((el) => el.classList.contains('selected'))).toEqual([true, false]);
    });
    it('should set the selected signature when you click on one that is not currently selected', () => {
      spyOn(Actions, 'selectSignature');
      this.item = ReactTestUtils.scryRenderedDOMComponentsWithClass(this.component, 'list-item')[1];
      ReactTestUtils.Simulate.click(this.item);
      expect(Actions.selectSignature).toHaveBeenCalledWith('2');
    });
    it('should modify the signature title when edited', () => {
      spyOn(Actions, 'upsertSignature');
      const input = (ReactDOM.findDOMNode(this.component) as HTMLElement).querySelector(
        '#signature-title'
      ) as HTMLInputElement;
      input.value = 'Changed';
      ReactTestUtils.Simulate.change(input);
      const [sig, id] = (Actions.upsertSignature as unknown as jasmine.Spy).mostRecentCall.args;
      expect(id).toBe('1');
      expect(sig.title).toBe('Changed');
      expect(sig.body).toBe(SIGNATURES['1'].body);
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
      const preview = ReactTestUtils.findRenderedDOMComponentWithClass(
        this.component,
        'signature-preview'
      );
      expect(preview.querySelector('#kept').textContent).toBe('Ann');
      expect(preview.innerHTML).not.toContain('onerror');
      expect(preview.innerHTML).not.toContain('<script');
      expect(preview.innerHTML).not.toContain('javascript:');
      expect((window as any).__sigXSS).toBe(false);
    });
  });

  describe('switching custom HTML back to a template', () => {
    let CUSTOM: { id: string; title: string; body: string; data: object };
    let editor: any;
    let answer: (button: number) => void;

    beforeEach(() => {
      CUSTOM = { id: '5', title: 'custom', body: '<b>my own html</b>', data: {} };
      spyOn(SignatureStore, 'getSignatures').andReturn({ '5': CUSTOM });
      spyOn(SignatureStore, 'selectedSignature').andReturn(CUSTOM);
      spyOn(SignatureStore, 'getDefaults').andReturn({});
      spyOn(Actions, 'upsertSignature');
      spyOn(Dialogs, 'ask').andCallFake(() => new Promise<number>((resolve) => (answer = resolve)));
      const component = makeComponent() as any;
      [editor] = ReactTestUtils.findAllInRenderedTree(
        component,
        (c: any) => !!(c && c._onDataFieldChange)
      );
    });

    const pickTemplate = () =>
      editor._onDataFieldChange({ target: { id: 'templateName', value: Templates[0].name } });

    it('asks once, app-modal, and applies the template on Continue', async () => {
      const first = pickTemplate();
      const second = pickTemplate();
      expect((Dialogs.ask as jasmine.Spy).callCount).toBe(1);
      expect((Dialogs.ask as jasmine.Spy).calls[0].args[1]).toBe('none');
      answer(1);
      await first;
      await second;
      const [sig, id] = (Actions.upsertSignature as unknown as jasmine.Spy).mostRecentCall.args;
      expect(id).toBe('5');
      expect(sig.data.templateName).toBe(Templates[0].name);
    });

    it('keeps the custom HTML on Cancel', async () => {
      const pending = pickTemplate();
      answer(0);
      await pending;
      expect(Actions.upsertSignature).not.toHaveBeenCalled();
    });
  });
});
