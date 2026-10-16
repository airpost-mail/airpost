import MailspringStore from 'mailspring-store';
import {
  Palette,
  RGBA,
  contrastRatio,
  formatColor,
  isDarkColor,
  parseColor,
} from '@tern/email-dark';
import { backgroundColorBehind } from './email-color-detection';

export const EMAIL_RENDER_MODE_KEY = 'core.reading.emailRenderMode';

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

// The theme colour around `el`: what a transparent email iframe there shows through to.
export function themeBackgroundAt(el: HTMLElement): RGBA {
  const [r, g, b, a] = backgroundColorBehind(el);
  return { r, g, b, a };
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
class OriginalColorsStore extends MailspringStore {
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
