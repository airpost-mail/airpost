import fs from 'fs';
import path from 'path';
import { Label } from 'mailspring-exports';
import { LabelColorStoreClass, LABEL_COLOR_SCHEMES } from '../../src/flux/stores/label-color-store';
import { LabelColorizer } from '../../src/components/mail-label';

const label = (path: string) => new Label({ path, accountId: 'a1' });

// -- WCAG contrast of the colours LabelColorizer asks Chromium for ------------------------------
// OKLab and OKLCH as in CSS Color 4; out-of-gamut colours are clipped per channel.

type Oklch = [number, number, number];

const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

function parseColor(css: string): number[] {
  if (css.startsWith('#')) {
    return [1, 3, 5].map((i) => parseInt(css.slice(i, i + 2), 16) / 255);
  }
  const [h, s, l] = /hsl\(([\d.]+),\s*([\d.]+)%,\s*([\d.]+)%\)/.exec(css).slice(1).map(Number);
  const a = (s / 100) * Math.min(l / 100, 1 - l / 100);
  return [0, 8, 4].map((n) => {
    const k = (n + h / 30) % 12;
    return l / 100 - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  });
}

function toOklch(css: string): Oklch {
  const [r, g, b] = parseColor(css).map(toLinear);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [L, Math.hypot(A, B), Math.atan2(B, A)];
}

function luminance([L, C, H]: Oklch) {
  const A = C * Math.cos(H);
  const B = C * Math.sin(H);
  const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
  const clip = (v: number) => Math.min(1, Math.max(0, v));
  const r = clip(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s);
  const g = clip(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s);
  const b = clip(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const contrast = (y1: number, y2: number) => (Math.max(y1, y2) + 0.05) / (Math.min(y1, y2) + 0.05);

const clamp = (min: number, v: number, max: number) => Math.min(max, Math.max(min, v));

/** The custom properties each `.label-*-bounds-light/-dark` mixin sets, read from the stylesheet itself. */
function bounds(mixin: string, dark: boolean): Record<string, number> {
  const file = path.join(
    AppEnv.getLoadSettings().resourcePath,
    'static/style/mixins/label-colors.less'
  );
  const less = fs.readFileSync(file).toString();
  const start = less.indexOf(`.${mixin}-${dark ? 'dark' : 'light'}() {`);
  const body = less.slice(start, less.indexOf('}', start));
  const out: Record<string, number> = {};
  for (const [, name, value] of body.matchAll(/--label-([\w-]+):\s*([\d.]+);/g)) {
    out[name] = Number(value);
  }
  return out;
}

// What a label colour can be: every scheme colour, and the Automatic hues every 15°.
const BASE_COLORS = [
  ...Object.values(LABEL_COLOR_SCHEMES).flat(),
  ...Array.from({ length: 24 }, (_, i) => `hsl(${i * 15}, 62%, 47%)`),
];

// The backgrounds dots sit on in the bundled themes: list and pane backgrounds and sidebars of
// ui-light, Taiga and Less is More (light); ui-dark and Darkside's sidebar (dark).
const LIGHT_SURFACES = ['#ffffff', '#f1f3f7', '#f4f6f9', '#fdfdfd'];
const DARK_SURFACES = ['#31343a', '#212121', '#111318', '#313042'];

describe('LabelColorStore', () => {
  // A store per spec: each spec gets a fresh AppEnv.config (master-before-each).
  let LabelColorStore: LabelColorStoreClass;

  beforeEach(() => {
    LabelColorStore = new LabelColorStoreClass();
  });

  it('keeps the hue derived from the name on Automatic', () => {
    const work = label('Work');
    expect(LabelColorStore.colorFor(work)).toBe(`hsl(${work.hue()}, 62%, 47%)`);
  });

  it('gives a label the same scheme colour in every account and letter case', () => {
    LabelColorStore.setScheme('earth');
    const color = LabelColorStore.colorFor(label('Clients/Acme'));
    expect(LABEL_COLOR_SCHEMES.earth).toContain(color);
    expect(LabelColorStore.colorFor(new Label({ path: 'clients/acme', accountId: 'a2' }))).toBe(
      color
    );
  });

  it("prefers the label's own colour to the scheme's, until it is reset", () => {
    LabelColorStore.setScheme('bright');
    const work = label('Work');
    const schemeColor = LabelColorStore.colorFor(work);
    LabelColorStore.setCustomColor(work, '#123456');
    expect(LabelColorStore.colorFor(work)).toBe('#123456');
    LabelColorStore.setCustomColor(work, null);
    expect(LabelColorStore.colorFor(work)).toBe(schemeColor);
  });

  it('notifies listeners when a colour changes', () => {
    const listener = jasmine.createSpy('listener');
    const unlisten = LabelColorStore.listen(listener);
    LabelColorStore.setCustomColor(label('Work'), '#123456');
    unlisten();
    expect(listener).toHaveBeenCalled();
  });

  it("carries a label's colour and its sublabels' through a rename", () => {
    LabelColorStore.setCustomColor(label('Clients'), '#111111');
    LabelColorStore.setCustomColor(label('Clients/Acme'), '#222222');
    LabelColorStore.setCustomColor(label('Clientsville'), '#333333');
    LabelColorStore.renameCustomColor('Clients', 'Customers');
    expect(LabelColorStore.customColor(label('Customers'))).toBe('#111111');
    expect(LabelColorStore.customColor(label('Customers/Acme'))).toBe('#222222');
    expect(LabelColorStore.customColor(label('Clients'))).toBe(null);
    expect(LabelColorStore.customColor(label('Clientsville'))).toBe('#333333');
  });

  it('keeps names with dots as single keys', () => {
    LabelColorStore.setCustomColor(label('v1.2'), '#123456');
    expect(LabelColorStore.customColor(label('V1.2'))).toBe('#123456');
  });
});

describe('LabelColorizer', () => {
  it('asks for colours Chromium can parse', () => {
    const el = document.createElement('div');
    document.body.appendChild(el);
    const work = label('Work');
    el.style.backgroundColor = LabelColorizer.backgroundColor(work);
    el.style.color = LabelColorizer.color(work);
    el.style.borderColor = LabelColorizer.backgroundColorDark(work);
    expect(el.style.backgroundColor).not.toBe('');
    expect(el.style.color).not.toBe('');
    expect(el.style.borderColor).not.toBe('');
    expect(getComputedStyle(el).backgroundColor).not.toBe('rgba(0, 0, 0, 0)');
    el.remove();
  });

  for (const dark of [false, true]) {
    const theme = dark ? 'dark' : 'light';

    it(`keeps chip text at 4.5:1 on ${theme} themes`, () => {
      const b = bounds('label-chip-bounds', dark);
      for (const base of BASE_COLORS) {
        const [L, C, H] = toOklch(base);
        const chip = luminance([
          clamp(b['chip-lmin'], L, b['chip-lmax']),
          Math.min(C, b['chip-cmax']),
          H,
        ]);
        const text = luminance([b['text-l'], Math.min(C, b['text-cmax']), H]);
        expect([base, contrast(chip, text) >= 4.5]).toEqual([base, true]);
      }
    });

    it(`keeps dots at 3:1 on ${theme} surfaces`, () => {
      const b = bounds('label-dot-bounds', dark);
      for (const base of BASE_COLORS) {
        const [L, C, H] = toOklch(base);
        const dot = luminance([clamp(b['dot-lmin'], L, b['dot-lmax']), C, H]);
        for (const surface of dark ? DARK_SURFACES : LIGHT_SURFACES) {
          const y = luminance(toOklch(surface));
          expect([base, surface, contrast(dot, y) >= 3]).toEqual([base, surface, true]);
        }
      }
    });
  }
});
