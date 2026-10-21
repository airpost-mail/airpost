/*
 * decaffeinate suggestions:
 * DS101: Remove unnecessary use of Array.from
 * DS102: Remove unnecessary code created because of implicit returns
 * DS207: Consider shorter variations of null checks
 * Full docs: https://github.com/decaffeinate/decaffeinate/blob/master/docs/suggestions.md
 */
// Utils for testing.
import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';

type MockObserver<T> = (data: T) => void;

const TestUtils = {
  loadKeymap: (path: string) => {
    return AppEnv.keymaps.loadKeymap(path);
  },

  simulateCommand: (target: EventTarget, command: string) => {
    return target.dispatchEvent(new CustomEvent(command, { bubbles: true }));
  },

  // React's "renderIntoDocument" does not /actually/ attach the component
  // to the document. It's a sham: http://dragon.ak.fbcdn.net/hphotos-ak-xpf1/t39.3284-6/10956909_1423563877937976_838415501_n.js
  // The Atom keymap manager doesn't work correctly on elements outside of the
  // DOM tree, so we need to attach it.
  renderIntoDocument(component: React.ReactElement) {
    // A function component has no instance, and a component that renders null has no node; the
    // walk up to the root below would throw on either.
    const node = ReactTestUtils.renderIntoDocument(component);
    const $node = node ? ReactDOM.findDOMNode(node) : null;
    if (!node || !$node) {
      throw new Error('TestUtils.renderIntoDocument: no instance or DOM node to attach');
    }
    if (!document.body.contains($node)) {
      let parent: Node = $node;
      while (parent.parentNode != null) {
        parent = parent.parentNode;
      }
      document.body.appendChild(parent);
    }
    return node;
  },

  removeFromDocument(reactElement: React.ReactInstance) {
    const $el = ReactDOM.findDOMNode(reactElement) as HTMLElement;
    if (document.body.contains($el)) {
      for (const child of Array.from(document.body.childNodes)) {
        if (child.contains($el)) {
          document.body.removeChild(child);
          return;
        }
      }
    }
  },

  // Returns mock observable that triggers immediately and provides helper
  // function to trigger later
  mockObservable<T>(data: T, param?: { dispose?: () => void } | null) {
    if (param == null) {
      param = {};
    }
    let { dispose } = param;
    if (dispose == null) {
      dispose = function () {};
    }
    let func: MockObserver<T> = function (_data: T) {};
    return {
      subscribe(fn: MockObserver<T>) {
        func = fn;
        func(data);
        return { dispose };
      },
      triggerNext(nextData?: T | null) {
        return func(nextData == null ? data : nextData);
      },
    };
  },
};
export default TestUtils;
