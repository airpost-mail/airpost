// What the modules bundled into the message frame import from 'mailspring-exports'. The
// frame bundle maps that specifier here (main/frame/protocol-handler.ts), since
// the real barrel loads Node and the whole app.
import DOMUtils from '../../utils/dom';
import RegExpUtils from '../../utils/regexp';

export { DOMUtils, RegExpUtils };

// Utils.escapeRegExp; data/models/utils needs fs.
export const Utils = {
  escapeRegExp(str: string) {
    return str.replace(/[-[\]/{}()*+?.\\^$|]/g, '\\$&');
  },
};
