import BaseStore from '../../data/stores/base';
import {
  Palette,
  RGBA,
  composite,
  contrastRatio,
  formatColor,
  isDarkColor,
  parseColor,
} from '@tern/email-dark';

export const EMAIL_RENDER_MODE_KEY = 'core.appearance.emailRenderMode';

export type EmailRenderMode = 'auto' | 'dark' | 'original';

export function emailRenderMode(): EmailRenderMode {
  const value = AppEnv.config.get(EMAIL_RENDER_MODE_KEY);
  if (value === 'dark' || value === 'original') return value;
  // 'light' was the opt-out of the earlier inverted dark rendering.
  if (value === 'light') return 'original';
  return 'auto';
}

// For 'dark' under a light theme, and for theme colours too faint to use as a dark palette.
export const DEFAULT_DARK_PALETTE: Palette = {
  background: '#1f1f1f',
  text: '#e8e8e8',
  link: '#8ab4f8',
};

const WHITE: RGBA = { r: 255, g: 255, b: 255, a: 1 };

// The colour a transparent email iframe at `el` shows through to: its ancestors' backgrounds
// composited up to the first opaque one (the message card, in the bundled themes).
export function themeBackgroundAt(el: HTMLElement): RGBA {
  const win = el.ownerDocument.defaultView;
  if (!win) return WHITE;
  const layers: RGBA[] = [];
  for (let node = el.parentElement; node; node = node.parentElement) {
    const color = parseColor(win.getComputedStyle(node).backgroundColor);
    if (!color || color.a === 0) continue;
    layers.push(color);
    if (color.a >= 1) break;
  }
  let result = WHITE;
  for (let i = layers.length - 1; i >= 0; i--) result = composite(layers[i], result);
  return result;
}

export function darkeningApplies(themeBackground: RGBA) {
  const mode = emailRenderMode();
  return mode === 'dark' || (mode === 'auto' && isDarkColor(themeBackground));
}

// Builds the darkening palette from the theme. `text` and `link` are the theme's email text
// and link colours; either falls back to the default when it isn't legible on the background.
export function darkPalette(themeBackground: RGBA, text: string, link: string): Palette {
  if (!isDarkColor(themeBackground)) return DEFAULT_DARK_PALETTE;
  const bg = { ...themeBackground, a: 1 };
  const legible = (value: string, min: number) => {
    const c = parseColor(value);
    return !!c && c.a === 1 && contrastRatio(c, bg) >= min;
  };
  return {
    background: formatColor(bg),
    text: legible(text, 4.5) ? text : DEFAULT_DARK_PALETTE.text,
    link: legible(link, 3) ? link : DEFAULT_DARK_PALETTE.link,
  };
}

// Messages the user switched back to the sender's colours, for this session.
class OriginalColorsStore extends BaseStore {
  _ids = new Set<string>();

  isShowingOriginal(messageId: string) {
    return this._ids.has(messageId);
  }

  setShowingOriginal(messageIds: string[], original: boolean) {
    for (const id of messageIds) {
      if (original) this._ids.add(id);
      else this._ids.delete(id);
    }
    this.trigger();
  }
}

export const OriginalColors = new OriginalColorsStore();
