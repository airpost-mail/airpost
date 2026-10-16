import {
  darkenDocument,
  removeDarkening,
  parseColor,
  relativeLuminance,
  contrastRatio,
  RGBA,
  Palette,
} from '@tern/email-dark';

const PALETTE: Palette = { background: '#212121', text: '#eeeeee', link: '#5aa3ff' };

// A typical marketing layout: bgcolor tables, a <style> block with an !important rule, inline
// styles, <font color>, an image, a gradient banner, a bordered card and an inline SVG icon.
const NEWSLETTER = `
<style>
  .header { background-color: #ffe0b2; }
  .headline { color: #202124; margin: 0; }
  .cta { background: #1a73e8; color: #ffffff !important; padding: 8px 16px; }
  .muted { color: #5f6368; font-size: 12px; }
</style>
<table id="outer" width="100%" bgcolor="#f4f4f4" cellpadding="0" cellspacing="0">
  <tr><td align="center">
    <table id="card" width="600" bgcolor="#ffffff" style="border: 1px solid #dddddd">
      <tr><td class="header" id="header"><h1 class="headline" id="headline">Spring catalogue</h1></td></tr>
      <tr><td id="body" style="color: #333333; font-size: 16px">
        New arrivals for the garden. <font id="sale" color="#d93025">Sale ends Friday.</font>
        <span id="pinned" style="color: #000000 !important">Prices include tax.</span>
      </td></tr>
      <tr><td><img id="hero" alt="Garden" width="600" height="200"
        src="data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACwAAAAAAQABAAACAkQBADs="></td></tr>
      <tr><td id="banner" style="background-image: linear-gradient(#ffffff, #eeeeee); color: #222222">
        <span id="banner-text">Free delivery this week</span>
      </td></tr>
      <tr><td><a class="cta" id="cta" href="https://shop.example.com/spring">Shop now</a></td></tr>
      <tr><td class="muted" id="muted">You get this email because you signed up at example.com.</td></tr>
      <tr><td><svg width="16" height="16"><path id="icon" d="M0 0h16v16H0z" fill="#000000"/></svg></td></tr>
    </table>
  </td></tr>
</table>`;

const DARK_EMAIL = `
<table id="outer" width="100%" bgcolor="#121212" cellpadding="0" cellspacing="0">
  <tr><td id="cell" style="color: #e0e0e0; padding: 40px; height: 300px">
    Your weekly summary is ready.
    <a href="https://dashboard.example.com" style="color: #8ab4f8">Open dashboard</a>
  </td></tr>
</table>`;

describe('email-dark', function () {
  let iframe: HTMLIFrameElement;
  let doc: Document;
  let wrapper: HTMLElement;

  const load = (html: string) => {
    iframe = document.createElement('iframe');
    iframe.setAttribute('sandbox', 'allow-same-origin');
    iframe.style.width = '700px';
    iframe.style.height = '900px';
    document.body.appendChild(iframe);
    doc = iframe.contentDocument;
    doc.open();
    doc.write(`<!DOCTYPE html><div id="wrapper">${html}</div>`);
    doc.close();
    wrapper = doc.getElementById('wrapper');
  };

  afterEach(() => {
    if (iframe) iframe.remove();
  });

  const style = (id: string) => doc.defaultView.getComputedStyle(doc.getElementById(id));
  const color = (value: string) => parseColor(value) as RGBA;
  const lum = (value: string) => relativeLuminance(color(value));

  describe('on a light newsletter', () => {
    beforeEach(() => {
      load(NEWSLETTER);
    });

    it('darkens backgrounds and keeps their order', () => {
      const result = darkenDocument(doc, PALETTE, { root: wrapper });
      expect(result.status).toBe('darkened');

      expect(style('card').backgroundColor).toBe('rgb(33, 33, 33)');
      expect(lum(style('outer').backgroundColor)).toBeLessThan(0.2);
      expect(lum(style('outer').backgroundColor)).toBeGreaterThan(
        lum(style('card').backgroundColor)
      );
      expect(style('wrapper').backgroundColor).toBe('rgb(33, 33, 33)');
    });

    it('keeps the hue of coloured backgrounds', () => {
      darkenDocument(doc, PALETTE, { root: wrapper });
      const header = color(style('header').backgroundColor);
      expect(relativeLuminance(header)).toBeLessThan(0.2);
      expect(header.r).toBeGreaterThan(header.g);
      expect(header.g).toBeGreaterThan(header.b);
    });

    it('lightens text set inline, in <style> blocks and with <font color>', () => {
      darkenDocument(doc, PALETTE, { root: wrapper });
      const header = color(style('header').backgroundColor);
      expect(contrastRatio(color(style('headline').color), header)).toBeGreaterThan(4.5);
      expect(lum(style('body').color)).toBeGreaterThan(0.5);
      expect(lum(style('muted').color)).toBeGreaterThan(0.2);
      expect(lum(style('muted').color)).toBeLessThan(lum(style('body').color));

      const sale = color(style('sale').color);
      expect(sale.r).toBeGreaterThan(sale.g + 40);
      expect(relativeLuminance(sale)).toBeGreaterThan(0.2);
    });

    it('overrides !important colours from <style> blocks and inline styles', () => {
      darkenDocument(doc, PALETTE, { root: wrapper });
      const cta = color(style('cta').color);
      const ctaBg = color(style('cta').backgroundColor);
      expect(contrastRatio(cta, ctaBg)).toBeGreaterThan(3);
      expect(lum(style('pinned').color)).toBeGreaterThan(0.5);
    });

    it('dims borders and recolours SVG icons', () => {
      darkenDocument(doc, PALETTE, { root: wrapper });
      expect(lum(style('card').borderTopColor)).toBeLessThan(0.1);
      expect(lum(style('icon').fill)).toBeGreaterThan(0.5);
    });

    it('leaves images and gradients alone, with the text on them', () => {
      darkenDocument(doc, PALETTE, { root: wrapper, imageBrightness: 0.9 });
      expect(doc.getElementById('hero').hasAttribute('data-email-dark')).toBe(false);
      expect(style('hero').filter).toBe('brightness(0.9)');
      expect(style('banner').backgroundImage).toContain('linear-gradient');
      expect(style('banner-text').color).toBe('rgb(34, 34, 34)');
    });

    it('removes every trace, also from a clone', () => {
      const pinned = doc.getElementById('pinned').getAttribute('style');
      darkenDocument(doc, PALETTE, { root: wrapper });

      const clone = doc.documentElement.cloneNode(true) as HTMLElement;
      removeDarkening(clone);
      expect(clone.querySelector('style[data-email-dark-sheet]')).toBe(null);
      expect(clone.querySelector('[data-email-dark]')).toBe(null);
      expect(clone.querySelector('#pinned').getAttribute('style')).toBe(pinned);
      // The live document is untouched by cleaning the clone.
      expect(style('card').backgroundColor).toBe('rgb(33, 33, 33)');

      removeDarkening(doc);
      expect(style('outer').backgroundColor).toBe('rgb(244, 244, 244)');
      expect(style('card').backgroundColor).toBe('rgb(255, 255, 255)');
      expect(style('pinned').color).toBe('rgb(0, 0, 0)');
      expect(doc.getElementById('pinned').getAttribute('style')).toBe(pinned);
      expect(doc.querySelector('[data-email-dark], [data-email-dark-root]')).toBe(null);
    });

    it('applies a flat palette past the element cap', () => {
      const result = darkenDocument(doc, PALETTE, { root: wrapper, maxElements: 5 });
      expect(result.status).toBe('fallback');
      expect(style('header').backgroundColor).toBe('rgba(0, 0, 0, 0)');
      expect(style('body').color).toBe('rgb(238, 238, 238)');
      expect(style('wrapper').backgroundColor).toBe('rgb(33, 33, 33)');
    });

    it('never adds scripts', () => {
      darkenDocument(doc, PALETTE, { root: wrapper });
      expect(doc.querySelectorAll('script').length).toBe(0);
    });
  });

  it('leaves an email that is already dark as it is', () => {
    load(DARK_EMAIL);
    const result = darkenDocument(doc, PALETTE, { root: wrapper });
    expect(result.status).toBe('already-dark');
    expect(style('outer').backgroundColor).toBe('rgb(18, 18, 18)');
    expect(style('cell').color).toBe('rgb(224, 224, 224)');
    expect(doc.querySelector('style[data-email-dark-sheet]')).toBe(null);
  });

  it('gives an email without colours the theme colours', () => {
    load('<p id="para">Hi Sam,</p><p>Lunch on Thursday?</p>');
    const result = darkenDocument(doc, PALETTE, { root: wrapper });
    expect(result.status).toBe('darkened');
    expect(style('wrapper').backgroundColor).toBe('rgb(33, 33, 33)');
    expect(style('para').color).toBe('rgb(238, 238, 238)');
  });
});
