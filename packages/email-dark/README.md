# @tern/email-dark

Renders an HTML email in a dark colour scheme, in place, the way Dark Reader's dynamic mode
does for web pages. Framework-free TypeScript; it needs a DOM `Document` (an iframe's, in
practice) and nothing else. Used by the desktop message view; written so `apps/web` can use it
too.

```ts
import { applyColorScheme, darkenDocument, removeDarkening } from '@tern/email-dark';

const result = darkenDocument(iframe.contentDocument, {
  background: '#212121', // the theme's colours
  text: '#eeeeee',
  link: '#5aa3ff',
}, { root: iframe.contentDocument.getElementById('body-wrapper'), imageBrightness: 0.9 });
// result.status: 'darkened' | 'already-dark' | 'fallback'

applyColorScheme(iframe, 'dark'); // before darkenDocument, after writing the document
removeDarkening(iframe.contentDocument); // or on a cloned tree before printing
```

How it works:

- One read pass over the subtree collects each element's computed background, text, border
  and SVG fill/stroke colours (so `bgcolor`, `<font color>`, `<style>` blocks and inline styles
  are all covered), skipping `display: none` and `opacity: 0` subtrees, scripts, styles and media.
- Backgrounds map to dark keeping hue and saturation (white lands on the theme background),
  text maps to light (black lands on the theme text colour), borders to dim lines. The mapping
  is ported from Dark Reader (MIT); see `src/color.ts` and NOTICE.md.
- Images, videos, background images and gradients are left alone, and text over a background
  image or gradient keeps the sender's colour. Images can be dimmed slightly.
- Emails designed dark are left untouched. The text votes: what most of it sits on (or, where
  the backdrop is unknown, whether it is light text). Emails with little text fall back to the
  share of painted area that is dark. Page the email doesn't paint doesn't vote, and a
  `<body>` colour set in the email's `<style>` counts even when the host hides it.
- Emails that declare a dark variant (`color-scheme` meta or CSS, `prefers-color-scheme: dark`
  rules) only need a quarter of the vote. `applyColorScheme(iframe, 'dark')` makes the frame's
  `prefers-color-scheme` follow the host theme instead of the OS, so their own dark styles are
  what gets measured and, usually, shown as they are.
- The write is one `<style>` element with a rule per distinct set of declarations, plus a
  `data-email-dark` attribute on each changed element. Inline `!important` declarations are
  overridden inline and restored by `removeDarkening`.
- Past `maxElements` (default 5000) it applies a flat palette override instead.
- Nothing from the email runs; no script is added.

Tests: `npm test` (transpiles with the TypeScript in `apps/desktop/node_modules`, runs
`node --test`). The DOM pass is covered by the desktop's Electron spec,
`apps/desktop/features/message-list/specs/email-dark-spec.ts`.

Licence: GPL-3.0-only.
