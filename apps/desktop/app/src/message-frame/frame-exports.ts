// What the modules bundled into the message frame import from 'mailspring-exports'. The
// frame bundle maps that specifier here (browser/message-frame-protocol-handler.ts), since
// the real barrel loads Node and the whole app.
import DOMUtils from '../dom-utils';
import RegExpUtils from '../regexp-utils';

export { DOMUtils, RegExpUtils };

// Utils.escapeRegExp; flux/models/utils needs fs.
export const Utils = {
  escapeRegExp(str: string) {
    return str.replace(/[-[\]/{}()*+?.\\^$|]/g, '\\$&');
  },
};
