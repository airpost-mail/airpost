# Desktop UI kit

Design for the desktop's component kit: Radix primitives, shadcn-like props and markup, plain CSS
reading the theme tokens (`docs/desktop/theme-tokens.md`). This is the spec stage 2 builds from;
nothing here is implemented yet except the dependencies and the compatibility spec
`tests/components/radix-react17.test.tsx`. Paths are relative to `apps/desktop/` unless they
start with `docs/`.

Settled before this doc (stage 0, `/tmp/tern-shots/ui-kit-stage0.md`): Radix per-primitive
packages, no Tailwind, no lucide, shadcn token names, native Electron context menus and
`dialog.showMessageBox` prompts stay, React 18 is a separate step. ContextMenu is not in the kit.

## 1. Packages

Runtime dependencies in `package.json`, all MIT with a CommonJS `main` (`./dist/index.js`) and a
peer range that includes React 17:

| Package | Version | Licence | Used by |
|---|---|---|---|
| `@radix-ui/react-slot` | 1.5.0 | MIT | Button `asChild` |
| `@radix-ui/react-switch` | 1.4.1 | MIT | Switch |
| `@radix-ui/react-checkbox` | 1.4.1 | MIT | Checkbox |
| `@radix-ui/react-radio-group` | 1.5.1 | MIT | RadioGroup |
| `@radix-ui/react-select` | 2.4.1 | MIT | Select |
| `@radix-ui/react-dialog` | 1.3.1 | MIT | Dialog |
| `@radix-ui/react-popover` | 1.3.1 | MIT | Popover |
| `@radix-ui/react-dropdown-menu` | 2.2.1 | MIT | DropdownMenu |
| `@radix-ui/react-tooltip` | 1.4.1 | MIT | Tooltip, IconButton |
| `@radix-ui/react-tabs` | 1.2.1 | MIT | Tabs |
| `@radix-ui/react-direction` | 1.2.0 | MIT | `UIProvider` on every React root |
| `@radix-ui/react-visually-hidden` | 1.2.13 | MIT | hidden Dialog titles |

They pull in 24 more `@radix-ui/*` internals (MIT) and these third-party packages: `@floating-ui/core` 1.8.0,
`@floating-ui/dom` 1.8.0, `@floating-ui/utils` 0.2.12, `@floating-ui/react-dom` 2.1.9 (all MIT),
`react-remove-scroll` 2.7.2, `react-remove-scroll-bar` 2.3.8, `react-style-singleton` 2.2.3,
`use-callback-ref` 1.3.3, `use-sidecar` 1.1.3, `aria-hidden` 1.2.6, `get-nonce` 1.0.1,
`detect-node-es` 1.1.0 (all MIT) and `tslib` 2.8.1 (0BSD). All GPL-3.0-compatible. On disk:
3.1 MB for `@radix-ui`, about 2.5 MB for the rest, including `.mjs`, `.d.ts` and source maps.
Installed with `npm install --ignore-scripts`; the native modules were left alone
(`scripts/check-native-abi.js` passes).

Requiring all twelve cold in Node takes about 50 ms (popover 15 ms, the rest 1-7 ms each). Each kit
file requires only its own primitive and the plugin API getters stay lazy, so a window only pays for
what it renders.

## 2. React 17 compatibility (verified)

- Types: a probe with shadcn-style `forwardRef` wrappers for every package, compiled with
  `strict` and `skipLibCheck: false` against `@types/react` 17.0.93, type-checks clean. A wrong prop
  type in the probe fails, so the `.d.ts` are really checked. The project `npm run typecheck`
  passes with the new spec.
- Runtime: `tests/components/radix-react17.test.tsx` (12 specs, Electron renderer, React 17.0.2)
  requires every package and renders Slot, Switch, Checkbox (indeterminate), RadioGroup (LTR and
  RTL arrows), Select (into a given portal container, picks a value), Dialog (title ids with no
  `React.useId`, no console errors), DropdownMenu (checkbox item, select closes), Tooltip, Tabs and
  Popover under `DirectionProvider`.
- Fallbacks read in the source: `react-id` uses a counter set in a layout effect when `useId` is
  missing; `use-effect-event`, `use-controllable-state` and `use-is-hydrated` fall back to
  `useLayoutEffect`/`useState` when `useInsertionEffect`, `useEffectEvent` or
  `useSyncExternalStore` are missing. Ids are set before paint, so the DOM is complete after
  `render()`.

Findings the spec pins down:

1. **Escape reaches the keymap.** Radix dismisses on a `keydown` listener on `document` in the capture
   phase and only calls `preventDefault()`. Mousetrap listens on `document` in the bubble phase and
   `stopCallback` (`src/window/keymap-manager.ts`) skips a binding only when
   `isPropagationStopped()`. With a plain Radix popover open, Escape closes it and also fires
   `core:pop-sheet`. With `onEscapeKeyDown={(e) => e.stopPropagation()}` it only closes the layer;
   the next Escape pops the sheet. Both cases run in the spec with real key events
   (`webContents.sendInputEvent`).
2. **Plain keys inside an open menu reach the keymap.** ArrowDown in an open Radix DropdownMenu moves
   the menu's highlight and also fires `core:next-item`. Enter would fire `core:focus-item`, Space
   `core:messages-page-down` and Backspace `core:remove-from-view`. The kit has to guard this (section 4.4).
3. **Popover content has no `dir` of its own.** Radix sets `dir` on RadioGroup, Tabs and the menu and
   select roots, but Popover and Dialog content inherit CSS direction from where they are portalled.
   The portal container must carry `dir` (section 4.1).
4. **RadioGroup's tab stop is the group, not the checked item.** At rest every item has `tabindex=-1`
   and the root has `tabindex=0`; focusing the root forwards to the checked item. Arrow keys move
   focus in a `setTimeout()`, and the item checks itself on that focus.
5. **The spec clock never fires `setTimeout(fn)` with no delay.** `TimeOverride._fakeSetTimeout`
   (`tests/support/runner/time-override.ts`) stores `now + undefined` = `NaN`. Radix uses delay-less
   timeouts (roving focus, menus, select). Kit specs call `jasmine.unspy(window, 'setTimeout')` and
   wait with `window.originalSetTimeout`, or use the helper stage 2 adds to `tests/support/test-utils.ts`.

## 3. Layout and imports

- Components: `src/components/ui/<name>.tsx`, one per component, kebab-case (`button.tsx`,
  `icon-button.tsx`, `switch.tsx`, `checkbox.tsx`, `radio-group.tsx`, `select.tsx`,
  `segmented-control.tsx`, `text-field.tsx`, `textarea.tsx`, `dialog.tsx`, `popover.tsx`,
  `dropdown-menu.tsx`, `tooltip.tsx`, `tabs.tsx`). Shared pieces: `portal.ts`, `layer.ts` (Escape and
  layer marking), `provider.tsx` (`UIProvider`), `cn.ts` if `classnames` isn't enough. No barrel
  file: core code imports the file it needs (`import { Button } from '../components/ui/button'`), so
  nothing requires all of Radix.
- Specs: `tests/components/kit-<name>.test.tsx`.
- Styles: `static/styles/kit/<name>.css` (section 5).
- Every component is a `React.forwardRef` function (React 17 doesn't pass `ref` as a prop), sets
  `displayName`, spreads the rest props onto the DOM part that carries `data-slot`, and merges
  `className` after its own classes. Files `import React from 'react'` (classic JSX runtime).
- Values are strings wherever Radix requires it (RadioGroup, Select, Tabs). Wrappers that take typed
  values (`ConfigSelectRow`, `SegmentedControl`) map with `String(v)` as they do today.
- Plugins: how the kit is exposed in the v1 plugin API is open (section 8). Core and the bundled
  `src/features/` import directly either way.

## 4. Shared behaviour

### 4.1 Portal container

`getPortalContainer(doc = document)` in `src/components/ui/portal.ts` returns one
`<div id="ui-portal-root" data-ui-portal-root>` per window, created on first use as the last
child of `<body>`. It gets `dir="rtl"` when `isRTL` (`src/shared/intl`). `<html>` already carries
`dir` in RTL (`src/window/app-env.ts` `initializeReactRoot`), so the container sits inside the
dir-carrying root and carries `dir` itself, so content portalled into it gets the right CSS
direction (finding 3). Every kit part that portals (Dialog, Popover, DropdownMenu, Select, Tooltip)
passes `container={getPortalContainer()}` unless the caller passes its own.

Stacking: the container is `position: relative; z-index: 1100; isolation: isolate`, above the
legacy modal container (`.modal-container`, z-index 1000) and FixedPopover (40), so a kit Select
inside a legacy modal opens on top. A legacy FixedPopover opened from inside a kit Dialog would open
underneath it. The migration batches don't create that combination; ModalStore and PopoverStore move
to the kit in the same stage (sections 6.10 and 6.11).

In specs the container is created under `<body>`, outside `#jasmine-content`: query
`document.body` or `getPortalContainer()`, not the render container. `cleanup()` unmounts the portal
content; the empty container may stay.

### 4.2 Providers on every React root

`UIProvider` (`src/components/ui/provider.tsx`) renders
`<DirectionProvider dir={isRTL ? 'rtl' : 'ltr'}>`. It wraps the four `ReactDOM.render` roots:

| Root | File |
|---|---|
| Workspace (`SheetContainer`) | `src/window/app-env.ts` `initializeReactRoot` |
| Modals | `src/data/stores/modal.tsx` (`renderModal` and the empty `<span />` renders) |
| Popovers | `src/data/stores/popover.tsx` (same) |
| Onboarding provider note | `src/features/onboarding/pages/create-page-for-form.tsx` |

Portals keep React context, so content portalled out of these roots still sees the direction. A
new `ReactDOM.render` call has to wrap its tree in `UIProvider` too. Stage 2 adds a spec that
greps `src/` and `plugins/` for `ReactDOM.render(` and fails on any call outside this list.
Tooltip carries its own `TooltipPrimitive.Provider` (shadcn does the same), so a tooltip can't throw
for lack of a provider in a root that forgot it. It just won't share the skip-delay between tooltips.

### 4.3 Escape

Rule: a kit layer that takes focus consumes the Escape that dismisses it. `layer.ts` exports

```ts
export function consumeEscape(handler?: (e: KeyboardEvent) => void) {
  return (e: KeyboardEvent) => {
    handler?.(e);
    // Radix only calls preventDefault(); stopping propagation keeps the keymap's
    // stopCallback (isPropagationStopped) from also firing core:pop-sheet.
    e.stopPropagation();
  };
}
```

`DialogContent`, `PopoverContent`, `DropdownMenuContent`, `DropdownMenuSubContent` and
`SelectContent` pass `onEscapeKeyDown={consumeEscape(props.onEscapeKeyDown)}`. Propagation is
stopped even when the caller calls `preventDefault()` to keep the layer open, so one Escape never
both keeps the layer and pops the sheet underneath it.

Details:

- Radix registers the listener only for the topmost layer, so nested layers close one per Escape.
  Stopping propagation at `document` in the capture phase doesn't stop the other layers' listeners
  on `document`; they aren't registered anyway.
- The event no longer reaches the focused element. Nothing inside a kit layer needs Escape: an input
  inside a popover loses its own Escape handling, and Escape closes the popover instead.
- A legacy `Modal` or `FixedPopover` underneath (React `onKeyDown` Escape handlers) doesn't close
  in the same keystroke. That's the intended layering.
- Tooltip doesn't consume Escape. It doesn't take focus, and today's native `title` tooltips don't
  block `core:pop-sheet`, so Escape closes the tooltip and pops the sheet as before.

### 4.4 Plain keys inside layers

Every focus-taking layer's content element carries `data-ui-layer` (Dialog, Popover, DropdownMenu
and its sub-content, Select). `stopCallback` in `src/window/keymap-manager.ts` gets one more rule,
next to `withinTextInput`:

```ts
const withinUILayer = !!element.closest('[data-ui-layer]');
if (withinUILayer && (isPlainKey || isReservedTextEditingShortcut)) return true;
```

Inside an open kit menu, select, popover or dialog, unmodified keys (arrows, Enter, Space,
Backspace, letters) don't fire global commands, and `mod+…` shortcuts still do. This only affects
elements the kit renders, so legacy behaviour is unchanged. Finding 2's spec flips to assert
`core:next-item` doesn't fire once the rule lands. Escape is covered by section 4.3; the rule
covers it as well while focus is inside the layer.

### 4.5 Focus and pointer

Radix defaults stay: Dialog and Popover move focus into the content on open and return it to the
trigger (or the previously focused element, for virtual anchors) on close. DropdownMenu and Select
are modal by default, like native menus: the outside doesn't take pointer events or wheel scroll,
and the rest of the page is `aria-hidden`. The first click outside only closes the menu. Popover
is non-modal by default.

## 5. CSS

- **Files.** `static/styles/kit/<name>.css`, plain CSS, one per component, plus `static/styles/kit/base.css` for
  shared rules (focus ring, the portal container, reduced motion).
- **Loading.** `ThemeManager.loadStaticStylesheets()` (`src/packages/theme-manager.ts`) loads
  them after `static/styles/index.less` through `requireStylesheet(<path>.css)`, from a fixed list
  in that file (`KIT_STYLESHEETS`, in dependency order). `.css` is read raw at priority -1. It skips
  LESS and rtlcss, and the theme-switch reload loop re-reads it like the rest of `static/`. Order:
  `static/styles/tokens/defaults.css` (-1), `index.less` (-1), kit (-1), package styles (0), theme tokens (1). On a
  specificity tie the kit beats core LESS, and plugins and themes beat the kit.
- **Not through LESS.** Kit CSS is never `@import`ed from a `.less` file. LESS 3.13 does arithmetic
  on bare slashes (`aspect-ratio: 16 / 9` compiles to `1.77777778`, so `grid-area: 1 / 2` and
  `font: 13px/1.4` break), and rtlcss would mirror physical properties a second time.
- **Tokens only.** Colours, radii, fonts and density come from `var(--…)` tokens in
  `static/styles/tokens/defaults.css`. Derived shades use `color-mix()` or relative colour syntax
  (`hsl(from var(--x) h s calc(l - 5))`). No LESS variables, no LESS colour functions, no new
  tokens, no colour literals other than `transparent` and `currentColor`. Shape follows the
  Preferences redesign: controls use `calc(var(--radius) * 2)` (6px at the default 3px), menus,
  popovers and dialogs `calc(var(--radius) * 8 / 3)` (8px). Control text is
  `calc(var(--font-size) - 1px)` (13px). Paddings are multiplied by `var(--density)`.
- **RTL.** Logical properties only: `margin-inline-start`, `padding-inline`, `inset-inline-end`,
  `border-start-start-radius`, `text-align: start`. Transforms that move along the inline axis (the
  switch thumb) flip with `:dir(rtl)`. Radix sets `data-side` on popper content from the resolved
  side, so placement CSS keys off `[data-side]`, not left or right.
- **Selectors.** Base rules select on `data-slot`, state on `data-state`, `data-disabled`,
  `data-highlighted`, `data-variant`, `data-size` and `aria-invalid`, e.g.
  `[data-slot='button'][data-variant='outline']`. Element-qualified selectors
  (`input[data-slot='input']`) are used where a legacy element rule has to be matched on
  specificity.
- **Legacy global rules.** `inputs.less` (`input[type='text']`, … and their `:focus`),
  `dropdowns.less` (`select`), `buttons.less` (`.btn`, `.btn-icon`, `.btn-toolbar`),
  `static/styles/components/switch.less` (`.slide-switch`), `static/styles/components/menu.less` (`.menu`, `.item`),
  `static/styles/components/modal.less` and `static/styles/components/fixed-popover.less` would leak into kit elements, either
  because they match the element or because the kit renders the legacy class (section 7). In the
  stage-2 commit that introduces a kit component, the core rules it would match get the guard
  `:where(:not([data-slot]))` on their root selector, e.g. `.btn:where(:not([data-slot]))`,
  `input[type='text']:where(:not([data-slot]))`. `:where()` adds no specificity, so every legacy
  element still matches with the same weight and nothing legacy changes. Kit elements always have
  `data-slot`, so they drop out. LESS 3.13 and rtlcss pass the guard through unchanged (checked).
  Guards go in core `static/styles/` only. Theme rules stay unguarded on purpose (section 7).
  Feature LESS that styles `.btn` in context (`.some-toolbar .btn`) is cleaned up by the batch
  that migrates that screen.
- **Motion.** `[data-state='open']` and `[data-state='closed']` get 120-150 ms fade/zoom keyframes.
  Radix Presence keeps closing content mounted until `animationend`. `@media (prefers-reduced-motion:
  reduce)` sets `animation: none`. Presence unmounts at once when the computed `animation-name` is
  `none`. Stage 2 checks whether spec windows load the kit sheets. If they do, specs that assert
  unmounting wait for `animationend`.
- **Focus ring.** `:focus-visible { outline: 2px solid var(--ring); outline-offset: 1px }`, the
  same as legacy `.btn`.
- The email iframe doesn't get the kit sheets; nothing in it renders kit components.

## 6. Components

Common to all: `className` and `style` pass through. Only parts that render DOM carry `data-slot`;
Radix `Root` parts that render nothing (Dialog, Popover, DropdownMenu, Select, Tooltip roots)
accept but drop it. "Legacy classes" are the classes rendered for theme compatibility (section 7).

### 6.1 Button

- Package: `@radix-ui/react-slot` (for `asChild`).
- Props: `React.ComponentPropsWithoutRef<'button'> & { variant?: 'default' | 'secondary' |
  'outline' | 'ghost' | 'destructive' | 'link' | 'toolbar'; size?: 'sm' | 'default' | 'lg';
  asChild?: boolean }`. `type` defaults to `"button"`. `toolbar` is a Tern addition for the window
  toolbar's buttons.
- Markup: `<button type="button" data-slot="button" data-variant="…" data-size="…">`.
- Sizes: `sm` 22px, `default` 26px, `lg` 32px high. Inline padding `calc(10px * var(--density))`,
  and 8/14px for sm/lg.
- Colours: default `--primary`/`--primary-foreground`; secondary `--secondary`/
  `--secondary-foreground`; outline `--control` with a `--control-border` border; ghost transparent,
  `--hover` on hover; destructive `--destructive`/`--destructive-foreground`; link `--link` and
  `--link-hover`, no box; toolbar `--control` on `--toolbar-background`, `--toolbar-foreground` text.
- Legacy classes: default `btn btn-emphasis`; secondary and outline `btn`; destructive
  `btn btn-danger`; toolbar `btn btn-toolbar`; ghost and link none. `sm` adds `btn-small` and `lg`
  adds `btn-large`. Disabled adds `btn-disabled` next to the native `disabled`.
- Replaces: `.btn` elements (about 164 sites), `.btn-toolbar` (33). `ButtonDropdown` (7) becomes a
  Button plus a DropdownMenu (split button) in its batch; the `ButtonDropdown` export keeps its
  API.

### 6.2 IconButton

- Package: none of its own; uses Tooltip.
- Props: `Omit<ButtonProps, 'children' | 'asChild' | 'variant'> & { icon?: string; iconUrl?:
  string; mode?: string; label: string; tooltip?: boolean; pressed?: boolean; variant?: 'ghost' |
  'outline' | 'secondary' | 'toolbar' }`. `icon` and `iconUrl` go to `RetinaImg` as `name` and
  `url`. `mode` defaults to `RetinaImg.Mode.ContentIsMask`. `label` is required: it is the
  `aria-label`, and also the tooltip unless `tooltip={false}`. `pressed` makes it a toggle button.
- Markup: `<button type="button" data-slot="icon-button" data-variant data-size aria-label
  [aria-pressed data-state="on|off"]><img class="content-mask" …></button>`. CSS sets
  `[data-slot='icon-button'] img.content-mask { background-color: currentColor }`, so the icon takes
  the button's text colour per state.
- Legacy classes: ghost `btn-icon`; toolbar `btn btn-toolbar`.
- Replaces: `.btn-icon` (11), `RetinaImg` with `onClick`/`onMouseDown` (12-16), `role="button"`
  divs (23), toolbar icon buttons.

### 6.3 Switch

- Package: `@radix-ui/react-switch`.
- Props: `Omit<ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>, 'onChange'> & { size?: 'sm' |
  'default'; onChange?: (event: React.SyntheticEvent) => void; label?: string; labelledBy?: string
  }`. Radix gives `checked`, `defaultChecked`, `onCheckedChange(checked: boolean)`, `disabled`,
  `required`, `name`, `value`. `onChange`, `label` and `labelledBy` are the legacy `Switch` props.
  `onChange` is called from `onClick` with the event, which keyboard activation (Space, Enter)
  also triggers on a `<button>`, so legacy callers that flip `checked` themselves keep working.
- Markup: `<button type="button" role="switch" aria-checked data-slot="switch"
  data-state="checked|unchecked" [data-disabled]><span data-slot="switch-thumb"
  data-state></span></button>`. Sizes: default 32×18px, sm 26×14px. The thumb moves with
  `translateX`, flipped under `:dir(rtl)`.
- Colours: track `--input` off and `--primary` on, thumb `--background`.
- Legacy classes: root `slide-switch`, plus `active` when checked; thumb `handle`.
- Replaces: `src/components/switch.tsx`, whose `Switch` export becomes this component with the same
  props; `SettingsSwitch` and `ConfigSwitchRow` in `src/components/settings.tsx`.

### 6.4 Checkbox

- Package: `@radix-ui/react-checkbox`.
- Props: `ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>`: `checked: boolean |
  'indeterminate'`, `defaultChecked`, `onCheckedChange(checked: boolean | 'indeterminate')`,
  `disabled`, `required`, `name`, `value`.
- Markup: `<button type="button" role="checkbox" aria-checked="true|false|mixed"
  data-slot="checkbox" data-state="checked|unchecked|indeterminate"><span
  data-slot="checkbox-indicator" data-state><svg …/></span></button>`. The check and dash glyphs
  are inline SVG paths drawn in `currentColor`; no icon package. A `<label htmlFor>` works because
  `<button>` is labelable.
- Colours: box `--control` with an `--input` border, checked `--primary`/`--primary-foreground`.
- Legacy classes: none (legacy checkboxes are native inputs).
- Replaces: the five native `type="checkbox"` sites (`src/features/preferences/tabs/mcp/section.tsx`,
  `src/features/onboarding/pages/account-settings-imap.tsx` ×2,
  `plugins/activity/lib/engagement/board.tsx`,
  `plugins/activity/lib/feed/toolbar.tsx`).

### 6.5 RadioGroup

- Package: `@radix-ui/react-radio-group`.
- Props: `RadioGroup` = `ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Root>` (`value`,
  `defaultValue`, `onValueChange(value: string)`, `disabled`, `required`, `name`, `orientation`,
  `loop`, `dir`); `RadioGroupItem` = `ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Item>`
  (`value`, `disabled`, `id`).
- Markup: `<div role="radiogroup" data-slot="radio-group" dir aria-orientation? tabindex="0">`;
  items `<button type="button" role="radio" aria-checked data-slot="radio-group-item"
  data-state="checked|unchecked" tabindex="-1"><span data-slot="radio-group-indicator"
  data-state></span></button>`. The group is the tab stop (finding 4). Arrow keys mirror in RTL.
- Legacy classes: none.
- Replaces: the two hand-rolled radio groups in
  `src/features/preferences/tabs/appearance.tsx` (theme tiles and label colours). Their
  tile visuals become the items' children.

### 6.6 Select

- Package: `@radix-ui/react-select`.
- Parts and props (shadcn names): `Select` (Root: `value`, `defaultValue`,
  `onValueChange(value: string)`, `open`, `onOpenChange`, `disabled`, `required`, `name`, `dir`);
  `SelectTrigger` (button props, `size?: 'sm' | 'default'`); `SelectValue` (`placeholder`);
  `SelectContent` (`position?: 'item-aligned' | 'popper'`, default `item-aligned` like the native
  macOS popup; `side`, `align`, `sideOffset` for popper; `container`); `SelectItem` (`value`,
  `disabled`, `textValue`); `SelectGroup`, `SelectLabel`, `SelectSeparator`,
  `SelectScrollUpButton`, `SelectScrollDownButton`.
- `SelectItem` must not have `value=""`: Radix throws, because the empty string means "cleared".
  Wrappers map an empty option value to a sentinel (`'__empty__'`) and back.
- Markup: trigger `<button type="button" role="combobox" aria-expanded aria-controls
  data-slot="select-trigger" data-size data-state="open|closed" [data-placeholder]><span
  data-slot="select-value">…</span><span data-slot="select-icon" aria-hidden/></button>`; content
  `<div role="listbox" data-slot="select-content" data-state data-side data-align data-ui-layer>`;
  item `<div role="option" aria-selected data-slot="select-item" data-state="checked|unchecked"
  [data-highlighted] [data-disabled]>` with `select-item-text` and `select-item-indicator`.
- Colours: trigger like an outline control (`--control`, `--input` border); content
  `--menu-background`, `--menu-foreground`, `--menu-border`, shadow from `--shadow-color`;
  highlighted item `--selected`/`--selected-foreground`.
- Legacy classes: none. Themes that restyle the `select` element no longer reach migrated selects.
- Replaces: native `<select>` (32 sites), starting with `ConfigSelectRow`'s `.settings-select`.
  This is a visible change on macOS: a DOM listbox instead of the native popup menu, with Radix's
  typeahead and arrow keys.

### 6.7 SegmentedControl

- Package: none. The current implementation in `src/components/settings.tsx` already does what's
  needed (`role="radiogroup"`, roving `tabIndex` with the chosen segment as the only tab stop, arrow
  keys that move the choice, focus follows once the parent applies the value), and
  `tests/components/settings.test.tsx` pins it. Radix RadioGroup would change the tab stop, make
  arrows asynchronous and force string values (findings 4 and 5), so the segmented control stays
  hand-written.
- Props: unchanged, `{ value: T; options: [T, string][]; onChange(value: T): void; disabled?:
  boolean; label?: string; className?: string }` with `T extends string | number | boolean`; adds
  `size?: 'sm' | 'default'`.
- Markup: unchanged roles and ARIA, plus `data-slot="segmented-control"` on the group and
  `data-slot="segmented-control-item" data-state="on|off"` on each segment.
- Legacy classes: kept, `settings-segmented` on the group and `on` on the chosen segment.
- Moves to `src/components/ui/segmented-control.tsx`; `settings.tsx` re-exports it, so the
  `SegmentedControl` export and `ConfigSegmentedRow` are unchanged. Its rules move from
  `static/styles/components/settings.less` to `static/styles/kit/segmented-control.css`.
- Not changed: arrows don't mirror in RTL today and still won't. Changing that is a behaviour change
  for a later stage.

### 6.8 TextField and Textarea

- Package: none.
- Props: `TextField` = `ComponentPropsWithoutRef<'input'> & { size?: 'sm' | 'default'; invalid?:
  boolean }`, `type` defaulting to `"text"`; `Textarea` = `ComponentPropsWithoutRef<'textarea'> & {
  invalid?: boolean }`. `invalid` sets `aria-invalid`.
- Markup: `<input data-slot="input" data-size [aria-invalid]>`, `<textarea data-slot="textarea"
  [aria-invalid]>`. The `data-slot` matches shadcn's `Input`.
- Colours: `--control`, `--control-foreground`, `--input` border, `--ring` on focus, `--destructive`
  when invalid, `--muted-foreground` placeholder.
- Legacy classes: none. The element is still `input[type=text]`, so theme rules on it keep applying
  (the core rules are guarded, section 5).
- Replaces: raw `<input>` (79) and `<textarea>` (7) in migrated screens. `TokenizingTextField` and
  `ParticipantsTextField` stay.
- Keymap: unchanged. These are `INPUT`/`TEXTAREA`, which `stopCallback` already treats as text
  input.

### 6.9 Dialog

- Package: `@radix-ui/react-dialog` (and `react-visually-hidden`).
- Parts and props: `Dialog` (Root: `open`, `defaultOpen`, `onOpenChange`, `modal` default true);
  `DialogTrigger` (`asChild`); `DialogContent` (Content props plus `showCloseButton?: boolean`
  default true, `hiddenTitle?: string`, `container?: HTMLElement`); `DialogHeader`, `DialogFooter`
  (div props); `DialogTitle`, `DialogDescription`, `DialogClose`.
- A dialog needs a title. Radix logs `console.error` without one. `hiddenTitle` renders a
  `DialogTitle` inside `VisuallyHidden` for dialogs with no visible heading. A dialog without a
  description passes `aria-describedby={undefined}`, which silences Radix's description warning.
- Markup: overlay `<div data-slot="dialog-overlay" data-state="open|closed">`; content
  `<div role="dialog" data-slot="dialog-content" data-state data-ui-layer aria-labelledby
  aria-describedby>`; `dialog-header`, `dialog-footer`, `dialog-title` (`h2`),
  `dialog-description` (`p`), `dialog-close`.
- Colours: overlay `--modal-overlay`; content `--modal-background`, `--modal-foreground`,
  `--modal-border`.
- Legacy classes: overlay `modal-container`, content `modal`, close button `modal-close`.
- Replaces: `Modal` (`src/components/modal.tsx`) through `ModalStore` (`src/data/stores/modal.tsx`).
  `Actions.openModal({ component, width, height })` and `Actions.closeModal()` keep their API
  (callers: `src/window/app-env.ts` and `src/features/theme-picker/index.tsx`). ModalStore renders `<Dialog open
  onOpenChange={(o) => !o && Actions.closeModal()}><DialogContent style={{ width, height }}
  hiddenTitle>`. The `Modal` export stays as a thin wrapper. Native `showMessageBox` and
  `showErrorDialog` prompts are untouched.

### 6.10 Popover

- Package: `@radix-ui/react-popover`.
- Parts and props: `Popover` (Root: `open`, `defaultOpen`, `onOpenChange`, `modal` default false);
  `PopoverTrigger` (`asChild`); `PopoverAnchor` (`asChild`, or `virtualRef` for a rect anchor);
  `PopoverContent` (`side`, `align`, `sideOffset` default 4, `collisionPadding` default 8,
  `avoidCollisions`, `onOpenAutoFocus`, `onEscapeKeyDown`, `onInteractOutside`, `container`);
  `PopoverArrow` (optional, for the legacy pointer look).
- Markup: content `<div role="dialog" data-slot="popover-content" data-state="open|closed"
  data-side data-align data-ui-layer>` inside Radix's `[data-radix-popper-content-wrapper]`
  (fixed position, `transform` set by floating-ui).
- Colours: `--menu-background`, `--menu-foreground`, `--menu-border`, shadow from `--shadow-color`.
- Legacy classes: content `fixed-popover`; arrow `fixed-popover-pointer`.
- Replaces: `FixedPopover` through `PopoverStore` (`src/data/stores/popover.tsx`).
  `Actions.openPopover(element, { originRect, direction, fallbackDirection, closeOnAppBlur })`
  keeps its API (24 callers). PopoverStore renders a `Popover open` with a `PopoverAnchor
  virtualRef` whose `getBoundingClientRect()` returns `originRect`. `direction` up/down/left/right
  maps to `side` top/bottom/left/right. `fallbackDirection` is covered by Radix's collision flip,
  which only flips to the opposite side. Stage 2 checks every caller's pair and lists any that asks
  for a perpendicular fallback. `closeOnAppBlur` maps to a `window` `blur` listener that closes.

### 6.11 DropdownMenu

- Package: `@radix-ui/react-dropdown-menu`.
- Parts and props (shadcn names): `DropdownMenu` (Root: `open`, `defaultOpen`, `onOpenChange`,
  `modal` default true, `dir`); `DropdownMenuTrigger` (`asChild`); `DropdownMenuContent` (`side`,
  `align`, `sideOffset` default 4, `loop`, `onCloseAutoFocus`, `onEscapeKeyDown`, `container`);
  `DropdownMenuItem` (`onSelect(event)`, `disabled`, `textValue`, `inset?`, `variant?: 'default' |
  'destructive'`); `DropdownMenuCheckboxItem` (`checked`, `onCheckedChange`);
  `DropdownMenuRadioGroup` (`value`, `onValueChange`) and `DropdownMenuRadioItem` (`value`);
  `DropdownMenuLabel` (`inset?`), `DropdownMenuSeparator`, `DropdownMenuShortcut` (span),
  `DropdownMenuGroup`, `DropdownMenuSub`, `DropdownMenuSubTrigger` (`inset?`),
  `DropdownMenuSubContent`.
- Markup: content `<div role="menu" aria-orientation="vertical" data-slot="dropdown-menu-content"
  data-state data-side data-align data-ui-layer>`; item `<div role="menuitem"
  data-slot="dropdown-menu-item" data-variant [data-inset] [data-highlighted] [data-disabled]>`;
  checkbox and radio items use `role="menuitemcheckbox|menuitemradio"`, `data-state="checked|unchecked"`
  and an indicator span; `dropdown-menu-separator` is `role="separator"`; the sub-trigger has
  `data-state="open|closed"`.
- Colours: as Select's content; destructive items `--destructive`.
- Legacy classes: content `menu`; items `item`, plus `selected` while highlighted (tracked from the
  item's `onFocus`/`onBlur`, since Radix highlights the focused item); separator `item divider`.
- Replaces: the legacy `DropdownMenu` component (2 sites), `ButtonDropdown`'s menu (7),
  `MultiselectDropdown` (1, as checkbox items). The legacy `DropdownMenu` export keeps its name and
  API. Not replaced: the searchable `Menu` (`src/components/menu.tsx`, used inside the category,
  label and snooze pickers), which stays legacy with its `.menu .item.selected` classes, and
  native context menus.

### 6.12 Tooltip

- Package: `@radix-ui/react-tooltip`.
- Parts and props: `Tooltip` (Root wrapped in its own Provider: `open`, `defaultOpen`,
  `onOpenChange`, `delayDuration` default 700, `disableHoverableContent`); `TooltipTrigger`
  (`asChild`); `TooltipContent` (`side` default `top`, `align`, `sideOffset` default 4,
  `container`).
- Markup: `<div data-slot="tooltip-content" data-state="delayed-open|instant-open|closed"
  data-side data-align>` plus Radix's visually hidden `role="tooltip"` copy for screen readers. No
  `data-ui-layer`, since it doesn't take focus.
- Colours: `--foreground` background, `--background` text (shadcn's inverted tooltip).
- Escape: not consumed (section 4.3).
- Legacy classes: none.
- Replaces: `title=` tooltips on migrated buttons; IconButton uses it for `label`. Native `title`
  stays everywhere else. How many of the 215 `title=` lines are tooltips is unknown.

### 6.13 Tabs

- Package: `@radix-ui/react-tabs`.
- Parts and props: `Tabs` (Root: `value`, `defaultValue`, `onValueChange`, `orientation`,
  `activationMode` default `automatic`, `dir`); `TabsList` (`loop`); `TabsTrigger` (`value`,
  `disabled`); `TabsContent` (`value`, `forceMount`).
- Markup: root `<div data-slot="tabs" dir data-orientation>`; list `<div role="tablist"
  data-slot="tabs-list" aria-orientation>`; trigger `<button role="tab" aria-selected
  aria-controls data-slot="tabs-trigger" data-state="active|inactive" data-orientation>`; panel
  `<div role="tabpanel" data-slot="tabs-content" data-state>`.
- Colours: list `--secondary`, active trigger `--background` with `--foreground`, inactive
  `--muted-foreground`.
- Legacy classes: none.
- Replaces: hand-rolled tabs in `src/components/composer-editor/emoji-toolbar-popover.tsx` and
  `plugins/activity/lib/root.tsx`. `src/features/preferences/sidebar.tsx` uses
  `role="tablist"` for a vertical nav with search, and moves to vertical Tabs only if its batch
  finds the semantics fit.

## 7. Theme compatibility during the deprecation window

Bundled themes are token-only, so they need nothing. Community themes that restyle classes (`.btn`,
`.btn-emphasis`, `.btn-toolbar`, `.btn-icon`, `.slide-switch`, `.menu .item.selected`, `.modal`,
`.fixed-popover`, `input[type='text']`) keep reaching migrated screens: each kit component also
renders the legacy root and state classes listed in section 6. Theme rules aren't guarded, so they
apply. Core rules for those classes are guarded (section 5), so they don't. What still breaks for
such themes: inner-DOM selectors (portals move content to `#ui-portal-root`), and selectors on the
old element types (`div.btn`, a theme's `select { … }` on a migrated Select). The legacy classes are
dropped at plugin API 2.0.0, together with the compat LESS path.

## 8. Needs a decision

1. **How plugins get the kit in the v1 API.** Two kit names collide with existing
   `mailspring-component-kit` exports: `Switch` and `DropdownMenu`. `Menu` and `Modal` exist as
   well but aren't kit names. Nothing existing may be renamed. Recommendation: one new lazy export,
   `UI`, holding every kit component under its shadcn name (`import { UI } from
   'mailspring-component-kit'; <UI.Button>`, `<UI.DropdownMenuContent>`), so nothing can collide
   now or later, and plugin API v2's types package can re-export the same names flat. The legacy
   `Switch` export becomes the kit Switch, whose props are a superset. `PLUGIN_API_VERSION` goes to
   1.34.0 when the first kit export ships, and `mailspring-component-kit.d.ts` plus
   `scripts/check-plugin-api-dts.js` gain the `UI` type. The alternative is flat additive names
   (`Button`, `Checkbox`, …) with `DropdownMenu` exposed only as parts plus a renamed root. That
   reads closer to shadcn, but gives the menu root an odd name forever.

## 9. Stage 2 checklist

- `src/components/ui/*` per section 6, `static/styles/kit/*.css` per section 5, `KIT_STYLESHEETS` in
  ThemeManager, `:where(:not([data-slot]))` guards on the core rules each component would match.
- `UIProvider` on the four roots; the `ReactDOM.render` allowlist spec.
- `consumeEscape` and `data-ui-layer` on every focus-taking layer; the `withinUILayer` rule in
  `stopCallback`; flip finding 2's spec.
- A spec per component under `tests/components/`, covering markup (`data-slot`, `data-state`,
  legacy classes), keyboard, the portal container, RTL, and Escape against `core:pop-sheet`.
- Rebuild `settings.tsx` on the kit (`SettingsSwitch`, `ConfigSelectRow`, `SegmentedControl` move),
  then ModalStore and PopoverStore on Dialog and Popover behind their actions. Both behind the same
  exports.
- A delay-less timer helper for specs (finding 5), or a fix in `TimeOverride` that treats a missing
  delay as 0 once the full suite shows nothing depends on the current never-fires behaviour.
