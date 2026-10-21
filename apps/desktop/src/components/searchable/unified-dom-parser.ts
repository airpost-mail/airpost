import { escapeRegExp } from '../../utils/regexp';
import { MAX_MATCHES, CHAR_THRESHOLD } from './constants';

export interface MatchNodeEntry {
  originalTextNode: any;
  newTextNodes: unknown[];
}

export default abstract class UnifiedDOMParser {
  // SearchableComponent renders once before it has a region id.
  regionId: string | undefined;
  matchRenderIndex: number;

  constructor(regionId: string | undefined) {
    this.regionId = regionId;
    this.matchRenderIndex = 0;
  }

  matchesSearch(dom: any, searchTerm: string | null): searchTerm is string {
    // CHAR_THRESHOLD is above 0, so an empty or null term is rejected here.
    if (!searchTerm || searchTerm.trim().length < CHAR_THRESHOLD) {
      return false;
    }
    const fullStrings = this.buildNormalizedText(dom);
    // For each match, we return an array of new elements.
    for (const fullString of fullStrings) {
      const matches = this.matchesFromFullString(fullString, searchTerm);
      if (matches.length > 0) {
        return true;
      }
    }
    return false;
  }

  buildNormalizedText(dom: unknown) {
    const walker = this.getWalker(dom);

    const fullStrings: unknown[][] = [];
    let textElementAccumulator: unknown[] = [];
    let stringIndex = 0;

    for (const node of walker) {
      if (this.isTextNode(node)) {
        (node as any).fullStringIndex = stringIndex;
        textElementAccumulator.push(node);
        stringIndex += this.textNodeLength(node);
      } else if (this.looksLikeBlockElement(node)) {
        if (textElementAccumulator.length > 0) {
          fullStrings.push(textElementAccumulator);
          textElementAccumulator = [];
          stringIndex = 0;
        }
      }
      // else continue for inline elements
    }
    if (textElementAccumulator.length > 0) {
      fullStrings.push(textElementAccumulator);
    }
    return fullStrings;
  }
  // OVERRIDE ME

  getWalker(dom: any): Iterable<unknown> {
    throw new Error('unimplemented');
  }
  isTextNode(node: any): boolean {
    throw new Error('unimplemented');
  }
  textNodeLength(textNode: any): number {
    throw new Error('unimplemented');
  }
  textNodeContents(textNode: any): string {
    throw new Error('unimplemented');
  }
  looksLikeBlockElement(node: any): boolean {
    throw new Error('unimplemented');
  }

  matchesFromFullString(fullString: any[], searchTerm: string) {
    const re = this.searchRE(searchTerm);
    if (!re) {
      return [];
    }
    const rawString = this.getRawFullString(fullString);
    const matches: Array<[number, number]> = [];
    let matchCount = 0;
    let match = re.exec(rawString);
    while (match && matchCount <= MAX_MATCHES) {
      const matchStart = match.index;
      const matchEnd = match.index + match[0].length;
      matches.push([matchStart, matchEnd]);
      match = re.exec(rawString);
      matchCount += 1;
    }
    return matches;
  }
  abstract getRawFullString(fullString: any): string;

  searchRE(searchTerm: string) {
    let re: RegExp;
    const regexRe = /^\/(.+)\/(.*)$/;
    try {
      if (regexRe.test(searchTerm)) {
        // Looks like regex
        // regexRe has no g flag, so match() finds what test() just found.
        const matches = searchTerm.match(regexRe)!;
        const reText = matches[1];
        re = new RegExp(reText, 'ig');
      } else {
        re = new RegExp(escapeRegExp(searchTerm), 'ig');
      }
    } catch (e) {
      return null;
    }
    return re;
  }

  // OVERRIDE ME
  abstract removeMatchesAndNormalize(element: any): unknown;

  getElementsWithNewMatchNodes(
    rootNode: any,
    searchTerm: string,
    currentMatchRenderIndex: number | null
  ) {
    const fullStrings = this.buildNormalizedText(rootNode);

    const modifiedElements = new Map<unknown, MatchNodeEntry>();
    // For each match, we return an array of new elements.
    for (const fullString of fullStrings) {
      const matches = this.matchesFromFullString(fullString, searchTerm);

      if (matches.length === 0) {
        continue;
      }

      for (const textNode of fullString) {
        const slicePoints = this.slicePointsForMatches(textNode, matches);
        if (slicePoints.length > 0) {
          const { key, originalTextNode, newTextNodes } = this.slicedTextElement(
            textNode,
            slicePoints,
            currentMatchRenderIndex
          );
          modifiedElements.set(key, { originalTextNode, newTextNodes });
        }
      }
    }

    return modifiedElements;
  }

  slicePointsForMatches(textElement: any, matches: Array<[number, number]>) {
    const textElStart = textElement.fullStringIndex;
    const textLength = this.textNodeLength(textElement);
    const textElEnd = textElement.fullStringIndex + textLength;

    const slicePoints: Array<[number, number]> = [];

    for (const [matchStart, matchEnd] of matches) {
      if (matchStart < textElStart && matchEnd >= textElEnd) {
        // textEl is completely inside of match
        slicePoints.push([0, textLength]);
      } else if (matchStart >= textElStart && matchEnd < textElEnd) {
        // match is completely inside of textEl
        slicePoints.push([matchStart - textElStart, matchEnd - textElStart]);
      } else if (matchEnd >= textElStart && matchEnd < textElEnd) {
        // match started in a previous el but ends in this one
        slicePoints.push([0, matchEnd - textElStart]);
      } else if (matchStart >= textElStart && matchStart < textElEnd) {
        // match starts in this el but ends in a future one
        slicePoints.push([matchStart - textElStart, textLength]);
      } else {
        // match is not in this element
        continue;
      }
    }
    return slicePoints;
  }

  /**
   * Given some text element and a slice point, it will split that text
   * element at the slice points and return the new nodes as a value,
   * keyed by a way to find that insertion point in the DOM.
   */
  slicedTextElement(
    textNode: any,
    slicePoints: Array<[number, number]>,
    currentMatchRenderIndex: number | null
  ) {
    const key = this.textNodeKey(textNode);
    const text = this.textNodeContents(textNode);
    const newTextNodes: unknown[] = [];
    let sliceOffset = 0;
    let remainingText = text;
    for (let [sliceStart, sliceEnd] of slicePoints) {
      sliceStart -= sliceOffset;
      sliceEnd -= sliceOffset;
      const before = remainingText.slice(0, sliceStart);
      if (before.length > 0) {
        newTextNodes.push(this.createTextNode({ rawText: before }));
      }

      const matchText = remainingText.slice(sliceStart, sliceEnd);
      if (matchText.length > 0) {
        let isCurrentMatch = false;
        if (this.matchRenderIndex === currentMatchRenderIndex) {
          isCurrentMatch = true;
        }
        newTextNodes.push(
          this.createMatchNode({
            regionId: this.regionId,
            renderIndex: this.matchRenderIndex,
            matchText,
            isCurrentMatch,
          })
        );
        this.matchRenderIndex += 1;
      }

      remainingText = remainingText.slice(sliceEnd, remainingText.length);
      sliceOffset += sliceEnd;
    }
    newTextNodes.push(this.createTextNode({ rawText: remainingText }));
    return {
      key: key,
      originalTextNode: textNode,
      newTextNodes: newTextNodes,
    };
  }
  // OVERRIDE ME

  abstract createTextNode(args: { rawText: string }): unknown;
  abstract createMatchNode(args: {
    matchText: string;
    regionId: string | undefined;
    isCurrentMatch: boolean;
    renderIndex: number;
  }): unknown;
  abstract textNodeKey(textElement: any): unknown;

  // OVERRIDE ME
  abstract highlightSearch(element: any, matchNodeMap: Map<unknown, MatchNodeEntry>): unknown;
}
