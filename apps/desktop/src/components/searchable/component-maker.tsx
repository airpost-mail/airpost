import React from 'react';
import ReactDOM from 'react-dom';
import * as Utils from '../../data/models/utils';
import VirtualDOMParser from './virtual-dom-parser';
import SearchableComponentStore from '../../data/stores/searchable-component';
import { Disposable } from 'rx-core';

type SuperMethod = ((...args: unknown[]) => unknown) | undefined;

class SearchableComponent {
  // Set in componentDidMount, so the first render runs without a region id.
  __regionId: string | undefined;
  __searchTerm: any;
  __searchIndex: any;
  _searchableListener?: () => void;

  componentDidMount(superMethod: SuperMethod, ...args: unknown[]) {
    if (superMethod) superMethod.apply(this, args);
    const regionId = Utils.generateTempId();
    this.__regionId = regionId;
    this._searchableListener = SearchableComponentStore.listen(() => {
      this._onSearchableComponentStoreChange();
    });
    SearchableComponentStore.registerSearchRegion(
      regionId,
      ReactDOM.findDOMNode(this as any) as HTMLElement
    );
  }

  _onSearchableComponentStoreChange() {
    const searchIndex = SearchableComponentStore.getCurrentRegionIndex(this.__regionId!);
    const { searchTerm } = SearchableComponentStore.getCurrentSearchData();
    (this as any).setState({
      __searchTerm: searchTerm,
      __searchIndex: searchIndex,
    });
  }

  shouldComponentUpdate(
    superMethod: ((nextProps: unknown, nextState: unknown) => boolean) | undefined,
    nextProps: unknown,
    nextState: { __searchTerm?: unknown; __searchIndex?: unknown }
  ) {
    let shouldUpdate = true;
    if (superMethod) {
      shouldUpdate = superMethod.apply(this, [nextProps, nextState]);
    }
    if (
      shouldUpdate &&
      (this.__searchTerm || (this.__searchIndex !== null && this.__searchIndex !== undefined))
    ) {
      shouldUpdate =
        this.__searchTerm !== nextState.__searchTerm ||
        this.__searchIndex !== nextState.__searchIndex;
    }
    return shouldUpdate;
  }

  // The lifecycle methods below run after componentDidMount, which sets both fields.
  componentWillUnmount(superMethod: SuperMethod, ...args: unknown[]) {
    if (superMethod) superMethod.apply(this, args);
    this._searchableListener!();
    SearchableComponentStore.unregisterSearchRegion(this.__regionId!);
  }

  componentDidUpdate(superMethod: SuperMethod, ...args: unknown[]) {
    if (superMethod) superMethod.apply(this, args);
    SearchableComponentStore.registerSearchRegion(
      this.__regionId!,
      ReactDOM.findDOMNode(this as any) as HTMLElement
    );
  }

  render(superMethod: SuperMethod, ...args: unknown[]) {
    if (superMethod) {
      const vDOM = superMethod.apply(this, args);
      const parser = new VirtualDOMParser(this.__regionId);
      const searchTerm = (this as any).state.__searchTerm;
      if (parser.matchesSearch(vDOM, searchTerm)) {
        const normalizedDOM = parser.removeMatchesAndNormalize(vDOM);
        const matchNodeMap = parser.getElementsWithNewMatchNodes(
          normalizedDOM,
          searchTerm,
          (this as any).state.__searchIndex
        );
        return parser.highlightSearch(normalizedDOM, matchNodeMap);
      }
      return vDOM;
    }
    return null;
  }
}

/**
 * Takes a React component and makes it searchable
 */
export class SearchableComponentMaker {
  static extend(component: typeof React.Component) {
    const proto = SearchableComponent.prototype;
    for (const propName of Object.getOwnPropertyNames(proto)) {
      const method: unknown = Reflect.get(proto, propName);
      const origMethod: unknown = Reflect.get(component.prototype, propName);
      if (origMethod) {
        if (propName === 'constructor' || typeof method !== 'function') {
          continue;
        }
        // What _.partial(method, origMethod) did: keep the dynamic `this`, prepend origMethod.
        Reflect.set(component.prototype, propName, function (this: unknown, ...args: unknown[]) {
          return method.apply(this, [origMethod, ...args]);
        });
      } else {
        Reflect.set(component.prototype, propName, method);
      }
    }
    return component;
  }

  static searchInIframe(contentDocument: Document) {
    return contentDocument;
  }
}
