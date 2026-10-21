import DOMUtils from '../../utils/dom';
import UnifiedDOMParser, { MatchNodeEntry } from './unified-dom-parser';

export default class RealDOMParser extends UnifiedDOMParser {
  *_pruningDOMWalker({
    node,
    pruneFn,
    filterFn,
  }: {
    node: Node;
    pruneFn: (node: Node) => boolean;
    filterFn: (node: Node) => boolean;
  }): Generator<Node, void, undefined> {
    if (filterFn(node)) {
      yield node;
    }
    if (node && !pruneFn(node) && node.childNodes.length > 0) {
      for (let i = 0; i < node.childNodes.length; i++) {
        yield* this._pruningDOMWalker({ node: node.childNodes[i], pruneFn, filterFn });
      }
    }
    return;
  }

  getWalker(dom: any): Iterable<Node> {
    const filterFn = (node: Node) => {
      return node.nodeType === Node.TEXT_NODE;
    };
    const pruneFn = (node: Node) => {
      return node.nodeName === 'STYLE';
    };
    return this._pruningDOMWalker({ node: dom, pruneFn, filterFn });
  }

  isTextNode(node: any) {
    return node.nodeType === Node.TEXT_NODE;
  }

  textNodeLength(textNode: any) {
    return (textNode.data || '').length;
  }

  textNodeContents(textNode: any) {
    return textNode.data;
  }

  looksLikeBlockElement(node: any) {
    return DOMUtils.looksLikeBlockElement(node);
  }

  getRawFullString(fullString: any) {
    return fullString.map((x: Text) => x.data).join('');
  }

  removeMatchesAndNormalize(element: any) {
    const matches = element.querySelectorAll('search-match');
    if (matches.length === 0) {
      return null;
    }
    for (let i = 0; i < matches.length; i++) {
      const match = matches[i];
      DOMUtils.unwrapNode(match);
    }
    element.normalize();
    return element;
  }

  createTextNode({ rawText }: any) {
    return document.createTextNode(rawText);
  }
  createMatchNode({ matchText, regionId, isCurrentMatch, renderIndex }: any) {
    const text = document.createTextNode(matchText);
    const newNode = document.createElement('search-match');
    const className = isCurrentMatch ? 'current-match' : '';
    newNode.setAttribute('data-region-id', regionId);
    newNode.setAttribute('data-render-index', renderIndex);
    newNode.setAttribute('class', className);
    newNode.appendChild(text);
    return newNode;
  }
  textNodeKey(textElement: any) {
    return textElement;
  }

  highlightSearch(element: any, matchNodeMap: Map<unknown, MatchNodeEntry>) {
    const walker = this.getWalker(element);
    // We have to expand the whole generator because we're mutating in
    // place
    const textNodes = [...walker];
    for (const textNode of textNodes) {
      if (matchNodeMap.has(textNode)) {
        const { originalTextNode, newTextNodes } = matchNodeMap.get(textNode)!;
        const frag = document.createDocumentFragment();
        for (const newNode of newTextNodes as Node[]) {
          frag.appendChild(newNode);
        }
        // The walker yields text nodes below `element`, which all have a parent.
        textNode.parentNode!.replaceChild(frag, originalTextNode);
      }
    }
  }
}
