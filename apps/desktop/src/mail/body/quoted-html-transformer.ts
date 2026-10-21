import quoteStringDetector from './quote-string-detector';
import unwrappedSignatureDetector from './unwrapped-signature-detector';
const { FIRST_ORDERED_NODE_TYPE } = XPathResult;

// ChildNode.remove(), for nodes the DOM types only know as Node.
const removeNode = (node: Node) => {
  node.parentNode?.removeChild(node);
};

const isEmptyishTextContent = (el: Node) => {
  // either '' or '---' (often left over from sig / confidentiality notice removal)
  const trimmed = (el.textContent ?? '').trim();
  return trimmed === '' || /^-+$/.test(trimmed);
};

// Email bodies reach us as body fragments, but a fragment whose first element
// is <style>, <title> or <meta> is routed into <head> by the tree builder — and
// fragments are serialized back out of <body> alone. Marketing mail commonly
// leads with its whole stylesheet, so parsing has to be pinned to "in body"
// mode or the sheet is silently dropped between sanitization and display.
const isWholeDocument = (html: string) => /<\s?head\s?>/i.test(html) || /<\s?body[\s>]/i.test(html);

const looksLikeTrackingPixel = (img: HTMLElement) => {
  // we want to avoid hiding quoted text if the user has added an image beneath it, but only
  // if that image is more than 1px in size...
  const w = Number(img.getAttribute('width') || (img.style.width || '').replace('px', '') || 10000);
  const h = Number(
    img.getAttribute('height') || (img.style.height || '').replace('px', '') || 10000
  );
  return w <= 1 && h <= 1;
};

class QuotedHTMLTransformer {
  annotationClass = 'quoted-text-segment';

  hasQuotedHTML(html: string) {
    const doc = this._parseHTML(html);
    this._removeImagesStrippedByAnotherClient(doc);
    this._removeTrailingFootersAndWhitespace(doc);
    return this._findQuoteElements(doc).length > 0;
  }

  // Public: Removes quoted text from an HTML string
  //
  // If we find a quoted text region that is "inline" with the root level
  // message, meaning it has non quoted text before and after it, then we
  // leave it in the message.
  //
  // - `html` The string full of quoted text areas
  // - `options`
  //   - `keepIfWholeBodyIsQuote` Defaults false. If true, then it will
  //   check to see if the whole html body is a giant quote. If so, it will
  //   preserve it.
  //
  // Returns HTML without quoted text
  //
  removeQuotedHTML(html: string, options = { keepIfWholeBodyIsQuote: true }) {
    const doc = this._parseHTML(html);
    this._removeImagesStrippedByAnotherClient(doc);
    this._removeTrailingFootersAndWhitespace(doc);
    for (const el of this._findQuoteElements(doc)) {
      if (el) {
        removeNode(el);
      }
    }

    // It's possible that the entire body was quoted text anyway and we've removed everything.
    if (options.keepIfWholeBodyIsQuote) {
      if (!doc.body || !doc.children[0] || (doc.body.textContent ?? '').trim().length === 0) {
        return this._outputHTMLFor(this._parseHTML(html), { initialHTML: html });
      }
    }

    if (!doc.body) {
      return this._outputHTMLFor(this._parseHTML(''), { initialHTML: html });
    }

    for (const el of quoteStringDetector(doc)) {
      if (el && el !== doc.body) {
        removeNode(el);
      }
    }

    // after removing all the quoted text, delete any whitespace that appeared between blocks
    // so the email doesn't end with <br> x 50
    this._removeUnnecessaryWhitespace(doc);

    return this._outputHTMLFor(doc, { initialHTML: html });
  }

  _removeImagesStrippedByAnotherClient(doc: Document) {
    if (!doc.body) {
      return;
    }

    const result = doc.evaluate(
      "//img[contains(@alt,'removed by sender')]",
      doc.body,
      null,
      XPathResult.ANY_TYPE,
      null
    );
    const nodes: Node[] = [];

    // collect all the results and then remove them all
    // to avoid modifying the dom while using the xpath selector
    let node = result.iterateNext();
    while (node) {
      nodes.push(node);
      node = result.iterateNext();
    }
    nodes.forEach(removeNode);
  }

  _removeUnnecessaryWhitespace(doc: Document) {
    if (!doc.body) {
      return;
    }

    // Find back-to-back <br><br> at the top level and de-duplicate them. Note that
    // some emails contain TEXT<br>TEXT<br>TEXT, so the only ELEMENT children may be the <brs>
    const nodes = doc.body.childNodes;
    const extraTailBrTags: ChildNode[] = [];
    for (let i = nodes.length - 1; i >= 0; i--) {
      const curr = nodes[i];
      const next = nodes[i - 1];
      if (curr && curr.nodeName === 'BR' && next && next.nodeName === 'BR') {
        extraTailBrTags.push(curr);
      } else {
        break;
      }
    }
    for (const el of extraTailBrTags) {
      el.remove();
    }

    // Traverse down the tree of "last child" nodes to get the last child of the last child.
    // The deepest node at the end of the document.
    let lastOfLast = doc.body;
    while (lastOfLast.lastElementChild) {
      lastOfLast = lastOfLast.lastElementChild as HTMLElement;
    }

    // Traverse back up the tree - at each level, attempt to remove
    // whitespace from the last child and then remove the child itself
    // if it's completely empty. Repeat until a child has meaningful content,
    // then move up the tree.
    //
    // Containers with empty space at the end occur pretty often when we
    // remove the quoted text and it had preceding spaces.
    const removeTrailingWhitespaceChildren = (el: Node) => {
      while (el.lastChild) {
        const child = el.lastChild;
        if (child.nodeType === Node.TEXT_NODE) {
          if (isEmptyishTextContent(child)) {
            child.remove();
            continue;
          }
        }
        if (['BR', 'P', 'DIV', 'SPAN', 'HR'].includes(child.nodeName)) {
          removeTrailingWhitespaceChildren(child);
          // An element, by its nodeName
          if ((child as Element).childElementCount === 0 && isEmptyishTextContent(child)) {
            child.remove();
            continue;
          }
        }
        break;
      }
    };

    while (lastOfLast.parentElement) {
      lastOfLast = lastOfLast.parentElement;
      removeTrailingWhitespaceChildren(lastOfLast);
    }
  }

  appendQuotedHTML(htmlWithoutQuotes: string, originalHTML: string) {
    let doc = this._parseHTML(originalHTML);
    const quoteElements = this._findQuoteElements(doc);
    doc = this._parseHTML(htmlWithoutQuotes);
    for (let i = 0; i < quoteElements.length; i++) {
      const node = quoteElements[i];
      doc.body.appendChild(node);
    }
    return this._outputHTMLFor(doc, { initialHTML: originalHTML });
  }

  _parseHTML(text: string) {
    const domParser = new DOMParser();
    let doc: Document;
    try {
      doc = domParser.parseFromString(isWholeDocument(text) ? text : `<body>${text}`, 'text/html');
    } catch (error) {
      const errText = `HTML Parser Error: ${String(error)}`;
      doc = domParser.parseFromString(errText, 'text/html');
      AppEnv.reportError(error);
    }

    // As far as we can tell, when this succeeds, doc /always/ has at least
    // one child: an <html> node.
    return doc;
  }

  _outputHTMLFor(doc: Document, { initialHTML }: { initialHTML?: string } = {}) {
    if (!doc.body) {
      doc = this._parseHTML('');
    }
    if (initialHTML !== undefined && isWholeDocument(initialHTML)) {
      return doc.children[0].innerHTML;
    }
    return doc.body.innerHTML;
  }

  _findQuoteElements(doc: Document): Node[] {
    const parsers = [
      this._findGmailQuotes,
      this._findYahooQuotes,
      this._findBlockquoteQuotes,
      this._findQuotesAfterMessageHeaderBlock,
      this._findQuotesAfter__OriginalMessage__,
    ];

    let quoteElements: Node[] = [];
    for (const parser of parsers) {
      quoteElements = quoteElements.concat(parser.call(this, doc) || []);
    }

    // Find top-level nodes that look like a signature - some clients append
    // a signature block /beneath/ the quoted text and we need to count is as
    // quoted text as well — otherwise it gets considered an inline reply block.
    const unwrappedSignatureNodes = unwrappedSignatureDetector(doc, quoteElements);
    quoteElements = quoteElements.concat(unwrappedSignatureNodes);

    // Keep quotes that are followed by non-quote blocks (eg: inline reply text)
    const quoteElementSet = new Set(quoteElements);
    quoteElements = quoteElements.filter(
      (el) => !this._isElementFollowedByUnquotedElement(el, quoteElementSet)
    );

    return quoteElements;
  }

  _isElementFollowedByUnquotedElement(el: Node, quoteElementSet: Set<Node>) {
    const seen = new Set<Node>();
    let head: Node | null = el;

    while (head) {
      // advance to the next sibling, or the parent's next sibling
      while (head && !head.nextSibling) {
        head = head.parentNode;
      }
      if (!head || !head.nextSibling) {
        break;
      }
      head = head.nextSibling;

      // search this branch of the tree for any text nodes / images that
      // are not contained within a matched quoted text block. We mark
      // the subtree as "seen" because we traverse upwards, and would
      // re-evaluate the subtree on each iteration otherwise.
      const pile: Node[] = [head];
      let node: Node | undefined;

      while ((node = pile.pop())) {
        if (seen.has(node)) {
          continue;
        }
        if (quoteElementSet.has(node)) {
          continue;
        }
        if (node.childNodes) {
          pile.push(...node.childNodes);
        }
        // An <img>, by its nodeName
        if (node.nodeName === 'IMG' && !looksLikeTrackingPixel(node as HTMLElement)) {
          return true;
        }
        if (node.nodeType === Node.TEXT_NODE && !isEmptyishTextContent(node)) {
          return true;
        }
      }
      seen.add(head);
    }

    return false;
  }

  _findGmailQuotes(doc: Document) {
    // Gmail creates both div.gmail_quote and blockquote.gmail_quote. The div
    // version marks text but does not cause indentation, but both should be
    // considered quoted text.
    return Array.from(doc.querySelectorAll('.gmail_quote'));
  }

  _findYahooQuotes(doc: Document) {
    // Both Yahoo and AOL wrap their quotes in divs with classes that contain
    // the text yahoo_quoted
    return Array.from(doc.querySelectorAll('[class*="yahoo_quoted"]'));
  }

  _findBlockquoteQuotes(doc: Document) {
    return Array.from(doc.querySelectorAll('blockquote'));
  }

  _removeTrailingFootersAndWhitespace(doc: Document) {
    let els: Node[] | null = [];
    let iters = 0;
    while ((els = this._findTrailingFooter(doc))) {
      iters++;
      els.forEach(removeNode);
      this._removeUnnecessaryWhitespace(doc);
      if (iters > 20) {
        return;
      }
    }
  }

  _findTrailingFooter(doc: Document): Node[] | null {
    // Traverse from the body down the tree of "last" nodes looking for a
    // Confidentiality Notice, "To unsubscribe from this group", etc.
    // We strip these nodes because otherwise the quoted text logic
    // thinks that they are inline replies to quoted text.
    const footerRegexps = [
      /^Confidentiality Notice/i,
      /strictly confidential/i,
      /This email message is/i,
      /You received this message because/i,
    ];

    const hasQuote = !!doc.querySelector('.gmail_quote, blockquote');

    let head: Node | null = doc.body;
    while (head) {
      const tc = (head.textContent ?? '').trim();
      if (head.nodeType === Node.TEXT_NODE) {
        if (footerRegexps.find((r) => r.test(tc))) {
          return [head];
        }
      }

      // chop off Google groups unsubscribe instructions which are appended
      // to the end but annoyingly not in a container.
      if (
        tc === '.' &&
        head.previousSibling &&
        head.previousSibling.previousSibling &&
        (head.previousSibling.previousSibling.textContent ?? '').trim().startsWith('To unsubscribe')
      ) {
        return [head, head.previousSibling, head.previousSibling.previousSibling];
      }

      // chop off gmail_signature if the user has it configured to go at the absolute
      // bottom of the email, beneath the quote. Without a quote it must stay: Gmail can
      // put text typed below the greeting inside the signature div.
      if (
        hasQuote &&
        head.nodeName === 'DIV' &&
        (head as Element).classList.contains('gmail_signature')
      ) {
        return [head];
      }

      if (head.childNodes.length === 0 && tc === '') {
        head = head.previousSibling;
      } else {
        head = head.lastChild;
      }
    }
    return null;
  }

  _findQuotesAfter__OriginalMessage__(doc: Document) {
    // these are pulled from specific messages seen in the wild. I think that doing this
    // via Xpath is still more performant than writing code to traverse + examine?
    const originalMessageMarker = doc.evaluate(
      `//div[. = '-------- Original message --------'] |
       //div[. = '------ Original Message ------'] |
       //div[starts-with(., '-----Original Message-----')] |
       //i[. = '-------Original Message-------'] |
       //div[. = '---Original---']`,

      doc.body,
      null,
      FIRST_ORDERED_NODE_TYPE,
      null
      // The XPath selects elements
    ).singleNodeValue as Element | null;

    if (!originalMessageMarker) {
      return [];
    }

    return this._collectAllNodesBelow(originalMessageMarker);
  }

  _findQuotesAfterMessageHeaderBlock(doc: Document) {
    // This detector looks for a element in the DOM tree containing
    // three children: <b>Sent:</b> or <b>Date:</b> and <b>To:</b> and
    // <b>Subject:</b>. It then returns every node after that as quoted text.

    // Find a DOM node exactly matching <b>Sent:</b>
    const dateXPath = `
      //b[. = 'Sent:'] |
      //b[. = 'Date:'] |
      //b[. = 'Sent: '] |
      //b[. = 'Date: '] |
      //span[. = 'Sent: '] |
      //span[. = 'Date: '] |
      //span[. = 'Sent:'] |
      //span[. = 'Date:']`;
    const dateMarker = doc.evaluate(
      dateXPath,
      doc.body,
      null,
      FIRST_ORDERED_NODE_TYPE,
      null
    ).singleNodeValue;

    if (!dateMarker) {
      return [];
    }

    // check to see if the parent container also contains the other two
    const headerContainer = dateMarker.parentElement;
    if (!headerContainer) {
      return [];
    }
    let matches = 0;
    for (const node of Array.from(headerContainer.children)) {
      const tc = (node.textContent ?? '').trim();
      if (tc === 'To:' || tc === 'Subject:') {
        matches++;
      }
    }
    if (matches !== 2) {
      return [];
    }

    // got a hit! let's cut some text.
    const quotedTextNodes = this._collectAllNodesBelow(headerContainer);

    // Special case to add "From:" because it's often detatched from the rest of the
    // header fields. We just add it where ever it's located.
    const fromXPath = "//b[. = 'From:'] | //span[. = 'From:']| //span[. = 'From: ']";
    let from: Node | null = doc.evaluate(
      fromXPath,
      doc.body,
      null,
      FIRST_ORDERED_NODE_TYPE,
      null
    ).singleNodeValue;

    if (from) {
      if (from.nodeName === 'SPAN') {
        from = from.parentElement;
      }
      // A <span> under doc.body always has a parent element
      if (from) quotedTextNodes.push(from);
    }

    return quotedTextNodes;
  }

  _collectAllNodesBelow = (headerContainer: Element): Node[] => {
    // The headers container and everything past it in the document is quoted text.
    // This traverses the DOM, walking up the tree and adding all siblings below
    // our current path to the array.
    let head: Element | null = headerContainer;
    const results: Node[] = [];
    while (head) {
      results.push(head);
      while (head && !head.nextElementSibling) {
        head = head.parentElement;
      }
      if (head) {
        head = head.nextElementSibling;
      }
    }
    return results;
  };
}

export default new QuotedHTMLTransformer();
