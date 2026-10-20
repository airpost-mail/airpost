# Theme tokens

Themes are CSS custom properties on `:root`. The defaults, at the Light theme's values, are in
`app/static/style/tokens/defaults.css`. A bundled theme sets tokens and nothing else. Kit components
are plain CSS reading `var(--…)`; legacy LESS keeps compiling and reads the tokens through the
variables in `static/style/base/ui-variables.less` (mapping below). Community themes that only set
LESS variables get their tokens derived from those variables (the compat path).

Rules the defaults follow:

- Names follow shadcn and `apps/web/src/index.css`. Tern adds a few base tokens shadcn lacks, then
  surface, state and shape tokens. There are no per-component tokens.
- Where `ui-variables.less` derives one variable from another, the token is derived the same way, so
  a theme that sets `--foreground` also moves `--subtle-foreground`, `--muted-foreground`, `--accent`
  and so on. Custom properties resolve on the element that declares them, so themes must declare on
  `:root` too.
- `defaults.css` loads before theme tokens. Its platform blocks use `:root:where(:has(> body.platform-…))`,
  which ranks like `:root`, so any theme rule on `:root` wins by order.
- System accent: ThemeManager sets `--system-accent` and `--system-accent-dark` on `:root` while
  "use system accent" is on. `--primary` is `var(--system-accent, #1f70ec)` and `--selected` reads
  `--system-accent-dark` first. A theme keeps that behaviour by writing
  `--primary: var(--system-accent, <its colour>)`, the same convention LESS themes use for `@accent-primary` today.

## Loading

ThemeManager (`src/theme-manager.ts`) loads `tokens/defaults.css` at core priority (-1), then the
active theme's tokens at theme priority (1), before the theme's other stylesheets. A theme's tokens
are its `styles/tokens.css`, read as plain CSS. A theme declares its scheme in `package.json`:
`"scheme": "light" | "dark"`, and `"sidebarScheme"` when its sidebar differs (Darkside). ThemeManager
puts `theme-scheme-light|dark` and `theme-sidebar-scheme-light|dark` on `body`; core LESS that used
to branch on a colour's lightness (the label colour bounds in `components/extra.less`) branches on
those classes. Without a declaration the scheme is light.

The email iframe gets the token sheets with `email-frame.less` (`EmailFrameStylesStore`), so
`email-frame.less` reads tokens too.

### Compat path for LESS-variables themes

A theme with `styles/ui-variables.less` and no `styles/tokens.css` is a LESS-variables theme. For it,
ThemeManager puts `static/style/compat` ahead of `static/style` in the LESS import path, so the
theme's `@import 'base/ui-variables'` lands on `compat/base/ui-variables.less`: the var()-based
base, then the literal values the variables had before tokens. All LESS then compiles against
literals as before, and LESS colour functions on any variable keep working. Its tokens come from
`compat/tokens.less`, compiled after the literal base and the theme's file
(`compatTokensSource()`), which sets each token from its source variable (the mapping below), so
an unset variable gives the literal default and never `var(--itself)`. Tokens with no LESS source
keep their `defaults.css` derivation. The scheme comes from the lightness of `@background-primary`
and `@source-list-bg` unless `package.json` declares one. `@list-hover-bg` and
`@toolbar-background-color` don't feed tokens: rows never hovered at `@list-hover-bg`, and the
toolbar variable is still read as LESS. The theme picker previews compile against the same literal
base.

## Base tier

shadcn names.

| Token | Meaning | Default |
|---|---|---|
| `--background` | Window and content background | `#ffffff` |
| `--foreground` | Body text | `#231f20` |
| `--card` / `--card-foreground` | Raised content, e.g. message cards on the reading pane | `var(--background)` / `var(--foreground)` |
| `--popover` / `--popover-foreground` | Menus, popovers | `var(--secondary)` / `var(--foreground)` |
| `--primary` / `--primary-foreground` | Accent, and text on it | `var(--system-accent, #1f70ec)` / `#ffffff` |
| `--secondary` / `--secondary-foreground` | Secondary background | `#f4f6f9` / `var(--foreground)` |
| `--muted` | Off-white panels (composer action bar, attachments, calendar) | `#fdfdfd` |
| `--muted-foreground` | De-emphasised text, headings in lists | `color-mix(--foreground 60%, transparent)` |
| `--accent` / `--accent-foreground` | Subtle hover background | `color-mix(--foreground 5%, transparent)` / `var(--foreground)` |
| `--destructive` / `--destructive-foreground` | Destructive actions | `#d9534f` / `#ffffff` |
| `--border` | Dividers and borders | `color-mix(--secondary 90%, #000)` |
| `--input` | Input and select borders | `color-mix(hsl(from --background, l − 10) 90%, transparent)` |
| `--ring` | Focus ring | `var(--primary)` |

Tern additions:

| Token | Meaning | Default |
|---|---|---|
| `--subtle-foreground` | Secondary text, between foreground and muted (sidebar items, snippets) | `color-mix(--foreground 85%, transparent)` |
| `--heading-foreground` | Headings, toolbar text | `#3a3f47` |
| `--link` / `--link-hover` | Links. Not `--primary`: Light keeps links blue under a system accent | `#1f70ec` / `#1a5fcc` |
| `--control` / `--control-foreground` | Surface of form controls: text inputs, selects, default buttons | `#ffffff` / `var(--foreground)` |
| `--control-border` | Button outline | `var(--shadow-color)` |
| `--shadow-color` | Shadows and hairline shadow borders | `rgba(0, 0, 0, 0.15)` |

## State tier

| Token | Meaning | Default |
|---|---|---|
| `--hover` | Hover background | `var(--accent)` |
| `--selected` / `--selected-foreground` | Strong selection: focused row, active item, selected chip | `var(--system-accent-dark, color-mix(--primary, black 10%))` / `var(--background)` |
| focus ring | Use `--ring` | |
| `--unread` | The thread list's unread dot, drawn as a mask in this colour | `#258fe1`, the colour of the dot icon it replaced, so Light doesn't follow the system accent there |
| `--success` / `--warning` / `--danger` / `--info` | Status colours; `--info` also fills info banners (`@background-color-info`) | `#5cb346` / `#f0ad4e` / `var(--destructive)` / `#1c65d4` |

The sidebar, thread list and nav rail have their own state tokens, listed with their surface.

## Surface tier

Every surface token defaults to a base or state token, except where Light uses its own value today
(`--sidebar-background`, `--thread-list-border`, `--modal-overlay`).

| Surface | Tokens and defaults |
|---|---|
| Toolbar: the window toolbar strip and the frame around the nav-rail card | `--toolbar-background`: `var(--sidebar-background)` on macOS, `var(--background)` on Linux and Windows. `--toolbar-foreground`: `var(--heading-foreground)`. `--toolbar-muted-foreground`: `var(--subtle-foreground)`. `--toolbar-border`: `color-mix(--foreground 10%)`, 12% on Linux |
| Toolbar above each column | `--sidebar-toolbar-background`, `--thread-list-toolbar-background`, `--message-list-toolbar-background`, all `transparent`: painted over the toolbar, so the toolbar shows through unless a theme sets them. `var(--toolbar-background)` would cover the blurred-window gradient and LESS themes that paint `.sheet-toolbar-container` |
| Buttons in the sidebar's part of the toolbar (compose, activity) | `--sidebar-toolbar-button` (capsule fill on macOS; hover is the full colour, rest 85%): `var(--background)`. `--sidebar-toolbar-foreground` (button icons, at 80%, 90% pressed): `var(--heading-foreground)`. `--sidebar-toolbar-muted-foreground` (activity icon): `#586069`. For a sidebar darker than the rest of the toolbar (Darkside); Linux and Windows draw these buttons as tints, so only the icon tokens apply there |
| Nav rail | `--rail-background`: `var(--toolbar-background)`. `--rail-foreground` (icons): `var(--subtle-foreground)`. `--rail-border` (divider): `color-mix(--foreground 12%)`. `--rail-hover`: `color-mix(--foreground 6%)`. `--rail-selected`: `var(--background)`. `--rail-selected-foreground`: `var(--sidebar-selected-foreground)`. `--rail-card-inset`: `1`, a number that scales the inset card the sheets sit in beside the rail (4px margin, 1px border, 6px corners); `0` joins them flush to the rail and toolbar, for a theme whose rail and sidebar are one surface |
| Sidebar: folder list, calendar list, preferences and contacts sidebars | `--sidebar-background`: `#f1f3f7`. `--sidebar-inset-background` (sidebar inside the nav-rail card): `color-mix(--sidebar-background 50%, --background)`. `--sidebar-foreground`: `var(--subtle-foreground)`. `--sidebar-muted-foreground`: `var(--muted-foreground)`. `--sidebar-border` (edge against the content): `var(--border)`. `--sidebar-separator` (dividers inside the sidebar): `var(--sidebar-border)`. `--sidebar-hover`: `var(--hover)`. `--sidebar-selected`: `var(--sidebar-background)` (Light selects by text colour only). `--sidebar-selected-foreground`: `var(--selected)`. `--sidebar-control` / `-control-foreground` / `-control-placeholder` (text fields in the sidebar, the Preferences search): `var(--control)` / `var(--foreground)` / `#757575`, Chromium's placeholder grey |
| Thread list: every ListTabular list | `--thread-list-background`: `var(--background)`. `--thread-list-foreground`: `var(--foreground)`. `--thread-list-muted-foreground`: `var(--muted-foreground)`. `--thread-list-border`: `#dcdfe5`. `--thread-list-hover`: `hsl(from --thread-list-background, l − 5)`. `--thread-list-unread` / `-unread-foreground`: `var(--background)` / `var(--thread-list-foreground)`. `--thread-list-read-foreground` (read threads in the thread list only; Dark dims them): `var(--thread-list-foreground)`. `--thread-list-selected` / `-selected-foreground` / `-selected-border` (multi-selection, or the open thread while the list isn't focused): `color-mix(--selected 17%, --thread-list-background)` / `var(--thread-list-foreground)` / `color-mix(--selected 50%, --thread-list-background)`. `--thread-list-focused` / `-focused-foreground` / `-focused-border` (keyboard cursor row): `var(--selected)` / `var(--selected-foreground)` / `var(--thread-list-focused)`. `--thread-list-cursor` (4px stripe on the keyboard-cursor row): `var(--thread-list-focused)`. `--thread-list-focused-icon-whiten`: `1`, a number that scales the filters whitening image icons and hover actions on focused rows; a theme with a pale `--thread-list-focused` sets `0` |
| Reading pane: message list and its participant sidebar | `--message-list-background`: `var(--secondary)`. `--message-list-foreground`, `--message-list-muted-foreground`, `--message-list-border`: foreground, muted-foreground, border |
| Composer | `--composer-background`: `var(--background)`. `--composer-foreground`, `--composer-muted-foreground`, `--composer-border`: foreground, muted-foreground, border |
| Menus and popovers | `--menu-background`: `var(--popover)`. `--menu-foreground`: `var(--popover-foreground)`. `--menu-muted-foreground`: `var(--subtle-foreground)`. `--menu-border`: `var(--border)`. A selected menu item uses `--primary` / `--primary-foreground`, as today |
| Modals | `--modal-background`: `var(--background)`. `--modal-foreground`, `--modal-muted-foreground`, `--modal-border`: foreground, muted-foreground, border. `--modal-overlay`: `rgba(255, 255, 255, 0.58)` |

## Shape tier

| Token | Meaning | Default |
|---|---|---|
| `--radius` | Base radius; the sm/lg steps scale from it | `3px` |
| `--radius-sm` / `--radius-md` / `--radius-lg` | | `calc(--radius × 2/3)` = 2px / `var(--radius)` / `calc(--radius × 5/3)` = 5px |
| `--font-sans` | UI font | `'Inter', 'Helvetica', sans-serif`; `system-ui` on Windows |
| `--font-serif` / `--font-mono` | | Georgia stack / DejaVu Sans Mono stack |
| `--font-heading` | | `var(--font-sans)` |
| `--font-size` | Base UI text size | `14px` |
| `--density` | Padding multiplier for kit components. Legacy LESS paddings ignore it | `1` |
| `--row-height` / `--row-height-narrow` | Thread list row height, wide and narrow layouts | `36px` / `85px` |

`--font-weight-semi-bold` already exists as a custom property in `workspace.less` (600, 700 on
Windows) and stays there.

The row heights must stay plain lengths. ThreadList reads its row height in JS through
`DOMUtils.getWorkspaceCssNumberProperty('thread-list-item-height-wide' | '-narrow')`, which parses
the computed value of `--thread-list-item-height-wide` on `mailspring-workspace` as a number. A
`calc()` stays unevaluated in a custom property's computed value and fails that parse; a `var()` is
substituted, so wiring `--thread-list-item-height-wide: var(--row-height)` works. Nothing sets
those two properties yet.

## Mapping from LESS variables

Uses are occurrences and files across non-theme LESS (`static/`, `features/` and `plugins/`), counted on 2026-10-19 while other LESS edits were in progress, so treat them as rough
sizes. "≈" means the Light value shifts by an amount you can't see. Where several variables map to
one token, the compat path derives the token from the one marked "source".

Colour math: legacy LESS colour functions (`darken`, `fade`, `mix`…) can't take `var()`. The
concurrent conversion to `color-mix()` and relative colours has removed nearly all of them on mapped
variables. What remains on mapped variables: `fadeout(@input-bg, 40%)` in `onboarding.less`, and
`onboarding-reset.less`, which redefines its own variables and stays LESS.

### Colours

| LESS variable | Token | Uses | Notes |
|---|---|---|---|
| `@background-primary` | `--background` | 125 / 39 | Many call sites are surfaces; see "Surface call sites" |
| `@background-off-primary` | `--muted` | 9 / 6 | |
| `@background-secondary` | `--secondary` | 41 / 21 | Menu, popover and reading-pane call sites move to surface tokens |
| `@background-tertiary` | stays LESS | 2 / 2 | `#6d7987` in every theme |
| `@accent-primary` | `--primary` | 87 / 31 | Already `var(--system-accent, …)` |
| `@accent-primary-dark` | `--selected` | 3 / 2 | |
| `@component-active-color` | `--selected` (source) | 30 / 13 | |
| `@component-active-bg` | `--selected-foreground` | 5 / 2 | |
| `@text-color` | `--foreground` | 157 / 41 | |
| `@text-color-subtle` | `--subtle-foreground` | 115 / 47 | |
| `@text-color-very-subtle` | `--muted-foreground` | 126 / 40 | |
| `@text-color-heading` | `--heading-foreground` | 12 / 5 | |
| `@text-color-inverse` | `--primary-foreground` | 39 / 14 | |
| `@text-color-inverse-subtle` | `color-mix(in srgb, var(--primary-foreground) 80%, transparent)` | 4 / 2 | Inline, no token |
| `@text-color-inverse-very-subtle` | `color-mix(in srgb, var(--primary-foreground) 50%, transparent)` | 3 / 3 | Inline, no token |
| `@text-color-link` | `--link` | 28 / 21 | |
| `@text-color-link-hover` | `--link-hover` (source) | 5 / 4 | |
| `@text-color-link-active` | `--link-hover` | 4 / 2 | |
| `@text-color-search-match`, `@text-color-search-current-match` | stay LESS | 3 / 3 each | No theme sets them |
| `@border-color-divider` | `--border` (source) | 90 / 32 | |
| `@border-color-secondary` | `--border` | 9 / 7 | |
| `@border-color-primary` | `--input` ≈ | 5 / 3 | `#e6e6e6` becomes `#e6e6e6` at 90% alpha |
| `@input-border-color` | `--input` (source) | 8 / 5 | |
| `@dropdown-default-border-color` | `--input` | 1 / 1 | Same formula as `@input-border-color` |
| `@input-bg` | `--control` (source) | 10 / 7 | |
| `@btn-default-bg-color` | `--control` ≈ | 19 / 6 | `#fefefe` becomes `#fff`. Dark: `#2e2e2e`, equal to its `@input-bg` |
| `@dropdown-default-bg-color` | `--control` | 2 / 2 | `<select>`. Dark had `#404040` against `#2e2e2e` inputs |
| `@btn-default-text-color` | `--control-foreground` (source) | 21 / 8 | |
| `@dropdown-default-text-color` | `--control-foreground` | 1 / 1 | |
| `@btn-border-color` | `--control-border` | 40 / 10 | |
| `@input-bg-disabled` | stays LESS | 1 / 1 | |
| `@btn-icon-color` | stays LESS | 2 / 2 | `#919191` in every theme |
| `@btn-emphasis-bg-color` | stays LESS (`@blue`) for now | 27 / 4 | Mapping it to `--primary` would make emphasis buttons follow theme and system accent; undecided |
| `@btn-emphasis-text-color` | `--primary-foreground` | 4 / 2 | |
| `@btn-danger-bg-color` / `-text-color` | `--destructive` / `--destructive-foreground` | 3 / 1, 2 / 1 | |
| `@btn-action-bg-color` / `-text-color` | `--success` / `--foreground` | 3 / 1, 2 / 1 | |
| `@color-success` | `--success` | 9 / 6 | |
| `@color-warning` | `--warning` | 15 / 7 | |
| `@color-danger` | `--danger` (source) | 15 / 9 | |
| `@color-error` | `--danger` | 7 / 4 | Same value |
| `@color-info` | `--info` | 6 / 1 | |
| `@background-color-info` | `--info` ≈ | 3 | `--info` moved to `#1c65d4` (this variable's value) from `#1a5fcc` (`@color-info`, only used mixed at 15–60%) |
| `@background-color-success` / `-warning` / `-error` / `-pending` | stay LESS | 1, 0, 5, 1 | Notification banner fills, deliberately saturated. No theme sets them |
| `@toolbar-background-color` | stays LESS for now; the compat path ignores it | 9 / 2 | Focused windows never show it on any platform: every platform block repaints the toolbar. It only shows through the base `body.is-blurred` gradient on blurred macOS and Linux windows, so a token theme gets the Light gradient there; undecided whether blurred toolbars should use `--toolbar-background` |
| `@panel-background-color` | `--sidebar-background` | 6 / 3 | Its frame call sites (darwin `.sheet-toolbar-container`, `.nav-rail-frame`) move to `--toolbar-background` / `--rail-background` |
| `@source-list-bg` | `--sidebar-background` (source) | 8 / 7 | |
| `@source-list-inset-bg` | `--sidebar-inset-background` | 2 / 2 | |
| `@source-list-active-bg` | `--sidebar-selected` | 2 / 1 | |
| `@source-list-active-color` | `--sidebar-selected-foreground` | 11 / 2 | `nav-rail.less` active mask moves to `--rail-selected-foreground` |
| `@list-bg` | `--thread-list-background` | 9 / 4 | |
| `@list-border` | `--thread-list-border` | 5 / 2 | |
| `@list-hover-bg` | `--thread-list-hover` ≈ | 2 / 2 | l − 4 becomes l − 5. Rows already hover at l − 5 (`list-tabular.less`), and that is what the token holds |
| `@list-selected-bg` / `-color` / `-border` | `--thread-list-selected` / `-selected-foreground` / `-selected-border` | 5 / 2, 1 / 1, 3 / 1 | |
| `@list-focused-bg` / `-color` / `-border` | `--thread-list-focused` / `-focused-foreground` / `-focused-border` | 12 / 3, 2 / 2, 3 / 2 | |
| `@standard-shadow-color` | `--shadow-color` | 35 / 9 | `@standard-shadow`, `@standard-shadow-up` and `@shadow-border` are composites of it and pick the token up through it |
| `@background-gradient` | stays LESS | 4 / 2 | Swipe backgrounds. Taiga's `none` is lost |
| `@black`, `@white`, `@gray-base` … `@gray-lighter`, `@blue`, `@blue-dark`, `@blue-light` | stay LESS | | Palette. ui-dark inverts `@white` and the greys, so its token-only version shows the Light greys at those call sites; see "Palette call sites" |
| `@open-tracking-color`, `@link-tracking-color`, `@spec-test-variable` | stay LESS | | |
| `@text-color-selected`, `@border-color-tertiary`, `@input-border-color-blurred`, `@menu-item-color-hover`, `@menu-item-color-selected`, `@menu-text-color-selected` | none | 0 | Unused outside themes |

### Radius, fonts, sizes

| LESS variable | Token | Uses | Notes |
|---|---|---|---|
| `@border-radius-small` | `--radius-sm` | 12 / 6 | |
| `@border-radius-base` | `--radius-md` | 72 / 27 | `buttons.less` `@border-radius-base + 1` becomes `calc(var(--radius-md) + 1px)` |
| `@border-radius-large` | `--radius-lg` | 9 / 6 | |
| `@component-border-radius` | none | 0 | |
| `@font-family` | `--font-sans` (source) | 5 / 4 | |
| `@font-family-sans-serif` | `--font-sans` | 3 / 1 | |
| `@font-family-heading` | `--font-heading` | 3 / 2 | |
| `@font-family-monospace` | `--font-mono` | 4 / 4 | |
| `@font-family-serif` | `--font-serif` | 1 / 1 | |
| `@font-size` | `--font-size` | 2 / 2 | No math at its call sites |
| `@font-size-base` and the sizes derived from it, `@line-height-*`, `@spacing-*`, `@padding-*`, `@component-padding` / `-icon-*` / `-line-height`, `@toolbar-height-darwin`, `@toolbar-btn-height-darwin`, `@input-font-size` | stay LESS | | Used in LESS arithmetic (`* 0.93`, `floor()`, `/ 2`) |
| `@font-weight-*` | stay LESS | | `@font-weight-semi-bold` is already `var(--font-weight-semi-bold)` |

### Surface call sites

These read a generic variable today but belong to a surface. Wiring changes them to the surface
token, so a theme can recolour the surface alone:

- Toolbar: `workspace.less` `.sheet-toolbar-container` and `.nav-rail-frame` backgrounds in the base
  rule and in every platform block (`@panel-background-color` on darwin, `@background-primary` on
  Linux and Windows) → `--toolbar-background`; the `.sheet-toolbar` hairline → `--toolbar-border`;
  `.sheet-toolbar-container` `color` → `--toolbar-foreground`.
- Column toolbars: `.toolbar-RootSidebar`, `.toolbar-CalendarSidebar` → `--sidebar-toolbar-background`;
  `.toolbar-ThreadList` → `--thread-list-toolbar-background`; `.toolbar-MessageList`,
  `.toolbar-MessageListSidebar` → `--message-list-toolbar-background`.
  Inside `.toolbar-RootSidebar` and `.toolbar-CalendarSidebar`, `.btn-toolbar` icon masks →
  `--sidebar-toolbar-foreground` and the macOS capsule → `--sidebar-toolbar-button`; the activity
  icon (`activity/styles/list.less`, was `@gray`) → `--sidebar-toolbar-muted-foreground`.
- Nav rail (`nav-rail.less`): `.nav-rail` → `--rail-background`; item mask → `--rail-foreground`;
  hover → `--rail-hover`; `.active` background → `--rail-selected`; `.active` mask →
  `--rail-selected-foreground`; `.nav-rail-divider` → `--rail-border`; `.nav-rail-restore` uses
  `--sidebar-foreground` and `--sidebar-hover`, since it sits over the sidebar; `.nav-rail-content`
  margin, border width and radius scale with `--rail-card-inset`.
- Sidebar (`outline-view.less`): `.item` text and mask (`@text-color-subtle`) → `--sidebar-foreground`;
  `.heading` (`@text-color-very-subtle`) → `--sidebar-muted-foreground`. This closes the gap where
  a dark sidebar got dark text.
- Thread list (`thread-list.less`): `.unread:not(.focused):not(.selected)` background
  (`@background-primary`) → `--thread-list-unread`, text → `--thread-list-unread-foreground`; read
  rows → `--thread-list-read-foreground`; the unread dot is a mask in `--unread`; `.inverseContent`
  (focused rows, split-mode selection) text and labels → `--thread-list-focused-foreground`, icon
  and hover-action filters scaled by `--thread-list-focused-icon-whiten`; `list-tabular.less` row
  hover (`darken(@list-bg, 5%)`) → `--thread-list-hover`, `.keyboard-cursor` stripe →
  `--thread-list-cursor`.
- Other sidebars: `.preferences-sidebar`, `.contacts-perspective-list` and `.calendar-source-list`
  read `--sidebar-background`, `--sidebar-foreground`, `--sidebar-muted-foreground` (labels,
  subtitles), `--sidebar-border`, `--sidebar-hover` and `--sidebar-selected-foreground` (active
  Preferences item). The Preferences separator reads `--sidebar-separator` and the search field
  `--sidebar-control`, `--sidebar-control-foreground` and `--sidebar-control-placeholder`. In Light the Preferences items go from `@text-color` to the sidebar's 85%
  text, the server card subtitle from 85% to 60%, and the active item from `--link` to `--selected`.
- Image icons: under `body.theme-scheme-dark`, `img.content-dark`, `img.content-light`
  (`extra.less`) and thread icons other than the unread dot and star (`thread-list.less`) are
  inverted, as ui-dark did. The rules sit in `:where()`, so a compat theme's own filter wins.
- Reading pane (`message-list.less`): `#message-list` and `.column-MessageListSidebar` backgrounds
  (`@background-secondary`) → `--message-list-background`; the message card → `--card`.
- Composer (`composer.less`): the composer's `@background-primary` backgrounds → `--composer-background`.
- Menus and popovers (`menu.less`, `fixed-popover.less`): `@background-secondary` →
  `--menu-background`; the header separator → `--menu-border`.
- Modals (`modal.less`): the `.animate` overlay `rgba(255, 255, 255, 0.58)` → `--modal-overlay`.

### Palette call sites

ui-dark flips the palette (`@white: #0a0a0a`, greys inverted). After it becomes token-only, these
call sites need a token if Dark should keep its look: `switch.less` (`@gray-light` track, `@white`
knob), `spinner.less` and `type.less` (`@gray-light`), `activity/list.less` (`@white`, `@gray`),
`composer.less` (`@gray-light`, `@gray`), `calendar.less` (`@gray-lighter`),
`attachment-items.less` (`@white`). Most of them map to `--muted-foreground`, `--control` or
`--primary-foreground`. Decide per site during the theme conversion.

## Darkside check

The palette-only Darkside prototype hit five gaps. The tokens that close them:

| Gap | Tokens |
|---|---|
| Sidebar text read `@text-color-subtle` | `--sidebar-foreground`, `--sidebar-muted-foreground` |
| macOS painted the whole toolbar and rail frame with the sidebar colour | `--toolbar-background`, `--rail-background`, the three column-toolbar tokens |
| Rail active item: page background behind a mask in the sidebar-selected colour (white on white) | `--rail-selected`, `--rail-selected-foreground` |
| Reading pane had no background of its own | `--message-list-background` |
| No success/warning | `--success`, `--warning` |

Darkside as tokens: `--sidebar-background: #313042`, `--sidebar-foreground: #9f9f9f`,
`--sidebar-selected-foreground: #ffffff`, `--sidebar-inset-background: var(--sidebar-background)`,
`--rail-background: var(--sidebar-background)`, `--rail-foreground: var(--sidebar-foreground)`,
`--rail-hover: rgba(255, 255, 255, 0.1)`, `--rail-selected-foreground: var(--primary)`,
`--toolbar-background: var(--background)`, `--sidebar-toolbar-background: var(--sidebar-background)`,
`--message-list-toolbar-background: var(--message-list-background)`, `--message-list-background: #f5f5f6`,
`--primary: #f18260` (no system accent: the old theme used its coral directly), `--thread-list-focused: #fef3ef`,
`--thread-list-focused-foreground: #313042`, `--thread-list-focused-icon-whiten: 0`,
`--thread-list-cursor: var(--primary)`, plus the prototype's border and status values.
The nav-rail card's inset and border are `--rail-card-inset`, which Darkside sets to `0` so the
rail and sidebar read as one column. `themes/ui-darkside/styles/tokens.css` has the shipped values.
