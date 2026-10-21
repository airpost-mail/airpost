import React from 'react';

import SearchMatch from './match';
import UnifiedDOMParser from './unified-dom-parser';

export interface VirtualDOMNode {
  element: unknown;
  parentNode: unknown;
  childOffset: unknown;
}

interface VirtualMatchNodeEntry {
  originalTextNode: VirtualDOMNode;
  newTextNodes: React.ReactNode[];
}

export const VirtualDOMUtils = {
  *walk({
    element,
    parentNode,
    childOffset,
    pruneFn = (element: any) => true,
  }: {
    element: unknown;
    parentNode?: unknown;
    childOffset?: unknown;
    pruneFn?: (element: any) => boolean;
  }): Generator<VirtualDOMNode, void, undefined> {
    yield { element, parentNode, childOffset };
    if (React.isValidElement(element) && !pruneFn(element)) {
      const children = (element.props as any).children;
      if (!children) {
        return;
      } else if (typeof children === 'string') {
        yield { element: children, parentNode: element, childOffset: 0 };
      } else if (children.length > 0) {
        for (let i = 0; i < children.length; i++) {
          yield* this.walk({ element: children[i], parentNode: element, childOffset: i, pruneFn });
        }
      } else {
        yield* this.walk({ element: children, parentNode: element, childOffset: 0, pruneFn });
      }
    } else if (element instanceof Array) {
      for (let i = 0; i < element.length; i++) {
        yield* this.walk({ element: element[i], parentNode: element, childOffset: i });
      }
    }
    return;
  },
};

export default class VirtualDOMParser extends UnifiedDOMParser {
  getWalker(dom: unknown): Iterable<VirtualDOMNode> {
    const pruneFn = (node: React.ReactElement) => {
      return node.type === 'style';
    };
    return VirtualDOMUtils.walk({
      element: dom,
      pruneFn,
      parentNode: undefined,
      childOffset: undefined,
    });
  }

  isTextNode({ element }: VirtualDOMNode): boolean {
    return typeof element === 'string';
  }

  textNodeLength({ element }: { element: string }) {
    return element.length;
  }

  textNodeContents(textNode: { element: string }) {
    return textNode.element;
  }

  looksLikeBlockElement({ element }: { element: unknown }) {
    if (!element) {
      return false;
    }
    const blockTypes = ['br', 'p', 'blockquote', 'div', 'table', 'iframe'];
    const type = typeof element === 'object' && 'type' in element ? element.type : undefined;
    if (typeof type === 'function') {
      return true;
    } else if (typeof type === 'string' && blockTypes.includes(type)) {
      return true;
    }
    return false;
  }

  getRawFullString(fullString: Array<{ element: string }>) {
    return fullString.map((x) => x.element).join('');
  }

  removeMatchesAndNormalize(element: any): React.ReactNode {
    // Pieces are only pushed while newChildren is still this array.
    const parts: React.ReactNode[] = [];
    let newChildren: React.ReactNode = parts;
    let strAccumulator: string[] = [];

    const resetAccumulator = () => {
      if (strAccumulator.length > 0) {
        parts.push(strAccumulator.join(''));
        strAccumulator = [];
      }
    };

    if (React.isValidElement(element) || Array.isArray(element)) {
      let children;

      if (Array.isArray(element)) {
        children = element;
      } else {
        children = (element.props as any).children;
      }

      if (!children) {
        newChildren = null;
      } else if (React.isValidElement(children)) {
        newChildren = children as any;
      } else if (typeof children === 'string') {
        strAccumulator.push(children);
      } else if (children.length > 0) {
        for (let i = 0; i < children.length; i++) {
          const child = children[i];
          if (typeof child === 'string') {
            strAccumulator.push(child);
          } else if (this._isSearchElement(child)) {
            resetAccumulator();
            parts.push(child.props.children);
          } else {
            resetAccumulator();
            parts.push(this.removeMatchesAndNormalize(child));
          }
        }
      } else {
        newChildren = children;
      }

      resetAccumulator();

      if (Array.isArray(element)) {
        return newChildren;
      }
      return React.cloneElement(element, {}, newChildren);
    }
    return element;
  }
  _isSearchElement(element: React.ReactElement) {
    return element.type === SearchMatch;
  }

  createTextNode({ rawText }: { rawText: string }) {
    return rawText;
  }
  createMatchNode({
    matchText,
    regionId,
    isCurrentMatch,
    renderIndex,
  }: {
    matchText: string;
    regionId: string | undefined;
    isCurrentMatch: boolean;
    renderIndex: number;
  }) {
    const className = isCurrentMatch ? 'current-match' : '';
    return React.createElement(SearchMatch, { className, regionId, renderIndex }, matchText);
  }
  textNodeKey(textElement: { parentNode: unknown }) {
    return textElement.parentNode;
  }

  highlightSearch(
    element: unknown,
    matchNodeMap: Map<unknown, VirtualMatchNodeEntry>
  ): React.ReactNode {
    if (React.isValidElement(element) || Array.isArray(element)) {
      // Pieces are only pushed while newChildren is still this array.
      const parts: React.ReactNode[] = [];
      let newChildren: React.ReactNode = parts;
      let children;

      if (Array.isArray(element)) {
        children = element;
      } else {
        children = (element.props as any).children;
      }

      const matchNode = matchNodeMap.get(element);
      let originalTextNode: VirtualDOMNode | null = null;
      let newTextNodes: React.ReactNode[] = [];
      if (matchNode) {
        originalTextNode = matchNode.originalTextNode;
        newTextNodes = matchNode.newTextNodes;
      }

      if (!children) {
        newChildren = null;
      } else if (React.isValidElement(children)) {
        if (originalTextNode && originalTextNode.childOffset === 0) {
          newChildren = newTextNodes;
        } else {
          newChildren = this.highlightSearch(children, matchNodeMap);
        }
      } else if (children instanceof Array && children.length > 0) {
        for (let i = 0; i < children.length; i++) {
          const child = children[i];
          if (originalTextNode && originalTextNode.childOffset === i) {
            parts.push(newTextNodes);
          } else {
            parts.push(this.highlightSearch(child, matchNodeMap));
          }
        }
      } else {
        if (originalTextNode && originalTextNode.childOffset === 0) {
          newChildren = newTextNodes;
        } else {
          newChildren = children;
        }
      }

      if (Array.isArray(element)) {
        return newChildren;
      }
      return React.cloneElement(element, {}, newChildren);
    }
    return element;
  }
}
