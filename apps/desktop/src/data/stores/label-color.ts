import BaseStore from './base';

export type LabelColorSchemeName = 'automatic' | 'bright' | 'pastel' | 'muted' | 'earth';

/** Twelve colours per scheme, in hue order; a label takes one by a hash of its name. */
export const LABEL_COLOR_SCHEMES: Record<Exclude<LabelColorSchemeName, 'automatic'>, string[]> = {
  bright: [
    '#e53935',
    '#fb8c00',
    '#fdd835',
    '#7cb342',
    '#43a047',
    '#00897b',
    '#00acc1',
    '#1e88e5',
    '#3949ab',
    '#8e24aa',
    '#d81b60',
    '#6d4c41',
  ],
  pastel: [
    '#f6a5a5',
    '#fbc99b',
    '#f7e38f',
    '#cfe8a4',
    '#a8dcb8',
    '#97d8cf',
    '#a3d9ea',
    '#a9c8f2',
    '#bcbdf0',
    '#d6b8ec',
    '#f2b3cf',
    '#d8c4b6',
  ],
  muted: [
    '#b56b6b',
    '#c08a5c',
    '#bfa95a',
    '#8fa362',
    '#6f9a7a',
    '#5f9a94',
    '#6593a6',
    '#6b82b0',
    '#7c76a8',
    '#9a76a6',
    '#b0708f',
    '#8a7b6e',
  ],
  earth: [
    '#a0522d',
    '#c97b3c',
    '#d4a843',
    '#8b8f3a',
    '#5f7f3f',
    '#3f6f5f',
    '#4f7f8a',
    '#4a6378',
    '#6b5b7b',
    '#8a4f5f',
    '#b5654a',
    '#7a6a55',
  ],
};

export const LABEL_COLOR_SCHEME_NAMES = [
  'automatic',
  ...Object.keys(LABEL_COLOR_SCHEMES),
] as LabelColorSchemeName[];

const SCHEME_KEY = 'core.appearance.labelColorScheme';
const COLORS_KEY = 'core.appearance.labelColors';

interface ColorableLabel {
  displayName: unknown;
  hue: () => number;
}

/** FNV-1a: stable across launches and devices, unlike the char-code sum behind `hue()`. */
function hashName(name: string) {
  let h = 0x811c9dc5;
  for (let i = 0; i < name.length; i++) {
    h ^= name.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function nameKey(label: ColorableLabel) {
  return typeof label.displayName === 'string' ? label.displayName.toLowerCase() : null;
}

/**
 * The colour of each label: the user's own colour for that label name, else the chosen
 * scheme's, else (Automatic) the hue derived from the name. Both settings are per device and
 * keyed by lowercased display name, so a label has one colour in every account.
 */
export class LabelColorStoreClass extends BaseStore {
  private _scheme: LabelColorSchemeName = 'automatic';
  private _colors: Record<string, string> = {};

  constructor() {
    super();
    this._load();
    AppEnv.config.onDidChange(SCHEME_KEY, this._onConfigChanged);
    AppEnv.config.onDidChange(COLORS_KEY, this._onConfigChanged);
  }

  // Every rendered chip listens (`MailLabel`), more than the default cap of 250.
  setupEmitter() {
    return super.setupEmitter().setMaxListeners(0);
  }

  scheme(): LabelColorSchemeName {
    return this._scheme;
  }

  setScheme(scheme: LabelColorSchemeName) {
    AppEnv.config.set(SCHEME_KEY, scheme);
  }

  customColor(label: ColorableLabel): string | null {
    const key = nameKey(label);
    return (key && this._colors[key]) || null;
  }

  /** `null` returns the label to the scheme's colour. */
  setCustomColor(label: ColorableLabel, color: string | null) {
    const key = nameKey(label);
    if (!key) return;
    const colors = { ...this._colors };
    if (color) {
      colors[key] = color;
    } else {
      delete colors[key];
    }
    AppEnv.config.set(COLORS_KEY, colors);
  }

  /** Carries the label's colour, and its sublabels', to the new name. */
  renameCustomColor(oldName: string, newName: string) {
    const from = oldName.toLowerCase();
    const to = newName.toLowerCase();
    if (from === to) return;
    const colors: Record<string, string> = {};
    let moved = false;
    for (const [key, color] of Object.entries(this._colors)) {
      if (key === from || key.startsWith(`${from}/`)) {
        colors[to + key.slice(from.length)] = color;
        moved = true;
      } else {
        colors[key] = color;
      }
    }
    if (moved) AppEnv.config.set(COLORS_KEY, colors);
  }

  schemeColor(label: ColorableLabel, scheme: LabelColorSchemeName = this._scheme) {
    const key = nameKey(label);
    const colors = scheme === 'automatic' ? null : LABEL_COLOR_SCHEMES[scheme];
    if (!colors || key === null) {
      return `hsl(${label.hue()}, 62%, 47%)`;
    }
    return colors[hashName(key) % colors.length];
  }

  /** Any CSS colour; chips and dots adapt it to the theme (`LabelColorizer`). */
  colorFor(label: ColorableLabel) {
    return this.customColor(label) || this.schemeColor(label);
  }

  _load() {
    const scheme = AppEnv.config.get(SCHEME_KEY);
    this._scheme = LABEL_COLOR_SCHEME_NAMES.includes(scheme) ? scheme : 'automatic';
    this._colors = { ...(AppEnv.config.get(COLORS_KEY) || {}) };
  }

  _onConfigChanged = () => {
    this._load();
    this.trigger();
  };
}

export default new LabelColorStoreClass();
