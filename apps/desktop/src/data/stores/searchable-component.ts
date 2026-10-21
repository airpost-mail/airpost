import _ from 'underscore';
import BaseStore from './base';
import DOMUtils from '../../utils/dom';
import * as Actions from '../actions';
import { MAX_MATCHES, CHAR_THRESHOLD } from '../../components/searchable/constants';
import FocusedContentStore from './focused-content';
import { Thread } from '../models/thread';

/**
 * A search region whose document the store can't read: a cross-origin message frame, which
 * highlights matches itself and reports their boxes relative to `element`.
 */
export interface RemoteSearchRegion {
  element: HTMLElement;
  matches: { top: number; left: number; height: number; renderIndex: number }[];
}

function isRemoteRegion(region: HTMLElement | RemoteSearchRegion): region is RemoteSearchRegion {
  return !!region && !(region instanceof Node) && 'element' in region;
}

/** A match's box; `top` is relative to the scroll ancestor once the matches are measured. */
interface SearchMatch {
  // Null for a match in a remote region.
  node: Element | null;
  regionId: string | null;
  renderIndex: number;
  top: number;
  left: number;
  height: number;
}

class SearchableComponentStore extends BaseStore {
  currentMatch: SearchMatch | null = null;
  matches: SearchMatch[] = [];
  globalIndex: number | null = null; // null means nothing is selected
  scrollAncestor: Element | null = null;

  // null and empty string are different. Null means that search isn't
  // even activated. Empty string means we're active but just not
  // searching anything.
  searchTerm: string | null = null;
  searchRegions: { [key: string]: HTMLElement | RemoteSearchRegion } = {};

  _lastThread?: Thread | null;

  constructor() {
    super();

    this._lastThread = FocusedContentStore.focused('thread');

    this.listenTo(Actions.findInThread, this._findInThread);
    this.listenTo(Actions.nextSearchResult, this._nextSearchResult);
    this.listenTo(Actions.previousSearchResult, this._previousSearchResult);
    this.listenTo(FocusedContentStore, () => {
      const newThread = FocusedContentStore.focused('thread');
      if (newThread !== this._lastThread) {
        this._findInThread(null);
        this._lastThread = newThread;
      }
    });
  }

  getCurrentRegionIndex(regionId: string) {
    if (regionId && this.currentMatch && this.currentMatch.regionId === regionId) {
      return this.currentMatch.renderIndex;
    }
    return null;
  }

  getCurrentSearchData() {
    return {
      searchTerm: this.searchTerm,
      globalIndex: this.globalIndex,
      resultsLength: this.matches.length,
    };
  }

  scrollbarTicks() {
    let ticks: (number | { percent: number; className: string })[] = [];
    const scrollAncestor = this.scrollAncestor;
    if (this.matches.length > 0 && scrollAncestor && scrollAncestor.scrollHeight > -1) {
      ticks = this.matches.map((match) => {
        if (match === this.currentMatch) {
          return {
            percent: match.top / scrollAncestor.scrollHeight,
            className: 'match',
          };
        }
        return match.top / scrollAncestor.scrollHeight;
      });
    }
    return ticks;
  }

  _nextSearchResult = () => {
    this._moveGlobalIndexBy(1);
  };

  _previousSearchResult = () => {
    this._moveGlobalIndexBy(-1);
  };

  // This needs to be debounced since it's called when all of our
  // components are mounting and unmounting. It also is very expensive
  // since it calls `getBoundingClientRect` and will trigger repaints.
  _recalculateMatches = _.debounce(() => {
    this.matches = [];

    // searchNodes need to all be under the root document. matches
    // may contain nodes inside of iframes which are not attached ot the
    // root document.
    const searchNodes: Node[] = [];

    if (this.searchTerm && this.searchTerm.length >= CHAR_THRESHOLD) {
      Object.entries(this.searchRegions).forEach(([regionId, region]) => {
        if (this.matches.length >= MAX_MATCHES) {
          return;
        }
        if (isRemoteRegion(region)) {
          if (!region.element.isConnected) return;
          searchNodes.push(region.element);
          const rect = region.element.getBoundingClientRect();
          for (const match of region.matches) {
            this.matches.push({
              node: null,
              regionId,
              renderIndex: match.renderIndex,
              top: match.top + rect.top,
              left: match.left + rect.left,
              height: match.height,
            });
            if (this.matches.length >= MAX_MATCHES) break;
          }
          return;
        }
        const node = region;
        let refNode: HTMLElement | Document;
        let topOffset = 0;
        let leftOffset = 0;
        if (node.nodeName === 'IFRAME') {
          searchNodes.push(node);
          const iframeRect = node.getBoundingClientRect();
          topOffset = iframeRect.top;
          leftOffset = iframeRect.left;
          // A cross-origin frame registers as a RemoteSearchRegion, so this one's document
          // is readable.
          const frameDocument = (node as HTMLIFrameElement).contentDocument!;
          refNode = frameDocument.body;
          if (!refNode) {
            refNode = frameDocument;
          }
        } else {
          refNode = node;
        }
        const matches = refNode.querySelectorAll<HTMLElement>('search-match, .search-match');
        for (let i = 0; i < matches.length; i++) {
          if (!DOMUtils.nodeIsLikelyVisible(matches[i])) {
            continue;
          }
          const rect = matches[i].getBoundingClientRect();
          if (node.nodeName !== 'IFRAME') {
            searchNodes.push(matches[i]);
          }
          this.matches.push({
            node: matches[i],
            regionId: matches[i].getAttribute('data-region-id'),
            renderIndex: Number(matches[i].getAttribute('data-render-index')),
            top: rect.top + topOffset,
            left: rect.left + leftOffset,
            height: rect.height,
          });
          if (this.matches.length >= MAX_MATCHES) {
            break;
          }
        }
      });
      this.matches.sort((nodeA, nodeB) => {
        const aScore = nodeA.top + nodeA.left / 1000;
        const bScore = nodeB.top + nodeB.left / 1000;
        return aScore - bScore;
      });

      if (this.globalIndex !== null) {
        this.globalIndex = Math.min(this.matches.length - 1, this.globalIndex);
        this.currentMatch = this.matches[this.globalIndex];
      }

      const parentFilter = (node: Node) => {
        const { classList } = node as Partial<Element>;
        return !!classList && classList.contains('scroll-region-content');
      };
      // The search nodes and the parents the filter keeps are all elements.
      const ancestor = DOMUtils.commonAncestor(searchNodes, parentFilter) as Element | null;
      const scrollAncestor = ancestor && ancestor.closest('.scroll-region-content');
      this.scrollAncestor = scrollAncestor;

      if (scrollAncestor) {
        const scrollRect = scrollAncestor.getBoundingClientRect();
        const scrollTop = scrollRect.top - scrollAncestor.scrollTop;
        // We save the position relative to the top of the scrollAncestor
        // instead of the current getBoudingClientRect (which is dependent
        // on the current scroll position)
        this.matches.forEach((match) => {
          match.top -= scrollTop;
        });
      }
    } else {
      this.currentMatch = null;
      this.globalIndex = null;
      this.scrollAncestor = null;
    }

    if (this.matches.length > 0) {
      if (this.globalIndex === null) {
        this._moveGlobalIndexBy(1);
      } else {
        this._scrollIntoView();
      }
    }

    this.trigger();
  }, 33);

  _moveGlobalIndexBy(amount: number) {
    if (this.matches.length === 0) {
      return;
    }
    let globalIndex: number;
    if (this.globalIndex === null) {
      globalIndex = 0;
    } else {
      globalIndex = this.globalIndex + amount;
      if (globalIndex < 0) {
        globalIndex += this.matches.length;
      } else {
        globalIndex = globalIndex % this.matches.length;
      }
    }
    this.globalIndex = globalIndex;
    this.currentMatch = this.matches[globalIndex];
    this._scrollIntoView();
    this.trigger();
  }

  _scrollIntoView() {
    if (!this.currentMatch || !this.scrollAncestor) {
      return;
    }

    const visibleRect = this.scrollAncestor.getBoundingClientRect();
    const scrollTop = this.scrollAncestor.scrollTop;
    const matchMid = this.currentMatch.top + this.currentMatch.height / 2;

    if (matchMid < scrollTop || matchMid > scrollTop + visibleRect.height) {
      const viewportMid = scrollTop + visibleRect.height / 2;
      const delta = matchMid - viewportMid;
      this.scrollAncestor.scrollTop = this.scrollAncestor.scrollTop + delta;
    }
  }

  _findInThread = (search: string | null) => {
    if (search !== this.searchTerm) {
      this.searchTerm = search;
      this.trigger();
      this._recalculateMatches();
    }
  };

  registerSearchRegion(regionId: string, domNode: HTMLElement | RemoteSearchRegion) {
    this.searchRegions[regionId] = domNode;
    this._recalculateMatches();
  }

  unregisterSearchRegion(regionId: string) {
    delete this.searchRegions[regionId];
    this._recalculateMatches();
  }
}
export default new SearchableComponentStore();
