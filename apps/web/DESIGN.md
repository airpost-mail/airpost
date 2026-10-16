# Tern web: design direction

Tern is a tool people sit in all day. It should feel like a well-made instrument: quiet,
exact, fast, and dense without being cramped. The reference points are Superhuman and
Linear for speed and keyboard flow; the character is our own.

**Direction: chart room.** Ink on cool chart paper by day, a night sea by night. One accent,
cobalt, taken from the tern mark. Hairline rules instead of boxes. One grotesque
throughout, tabular mono for numbers. No serif and no italics anywhere in the UI.
No gradients on surfaces, no glass, no drop shadows on resting UI. The one flourish in
the chrome is the cobalt hairline under the top bar.

Tokens live in `src/index.css` (`:root`, `.dark`, `@theme inline`). Use the Tailwind
names below; never hard-code colours or font families in components.

## Typography

| Role | Family                          | Tailwind                | Use                                   |
| ---- | ------------------------------- | ----------------------- | ------------------------------------- |
| UI   | Instrument Sans (variable, OFL) | `font-sans` (default)   | everything                            |
| Mono | JetBrains Mono (variable, OFL)  | `font-mono` + `tabular` | dates in lists, counts, key caps, ids |

Fonts are bundled through `@fontsource`; never load fonts from a CDN.

Scale (dense, 13px base):

| Token          | Size / line | Use                                                                 |
| -------------- | ----------- | ------------------------------------------------------------------- |
| `text-2xs`     | 11 / 16     | counts, list dates, section eyebrows (uppercase, `tracking-widest`) |
| `text-xs`      | 12 / 16     | secondary metadata, banners                                         |
| `text-sm`      | 13 / 20     | default UI text, list rows, menus (shadcn's default)                |
| `text-base`    | 14 / 22     | reading: plain-text bodies, composer                                |
| `text-xl`      | 20          | thread subject in the reader (`font-semibold tracking-tight`)       |
| `text-display` | 32 / 36     | screen titles and empty states (`font-semibold tracking-tight`)     |

Weight carries state: unread rows are `font-semibold`; read rows are regular. Do not use
bold for emphasis elsewhere in lists.

## Colour

Semantic tokens (light / dark values in `src/index.css`):

| Token                               | Meaning                                                                      |
| ----------------------------------- | ---------------------------------------------------------------------------- |
| `background`, `foreground`          | app canvas and ink                                                           |
| `sidebar`, `sidebar-accent`         | left pane and its hover/active row                                           |
| `topbar`                            | top bar surface                                                              |
| `reader`                            | reading pane surface (white "paper" in light, lifted navy in dark)           |
| `primary`                           | cobalt: primary actions, focus ring, links, selected markers                 |
| `unread`                            | unread dot and unread count emphasis                                         |
| `star`                              | starred (amber)                                                              |
| `row-hover`, `row-selected`         | thread list hover and the open thread                                        |
| `row-focus`                         | the keyboard cursor in a list (2px left rule) when it differs from selection |
| `hairline`                          | row separators: `border-hairline` (lighter than `border`)                    |
| `muted-foreground`                  | snippets, metadata                                                           |
| `success`, `warning`, `destructive` | connection state, sync errors, destructive actions                           |

Account colours (`AccountJSON.color`) appear only as a 6px dot next to the account name
and, in unified views, on rows. Never as fills.

Email bodies are always rendered on white, even in dark mode: senders design for white,
and inverting their HTML breaks it. The frame sits on the `reader` surface with rounded
corners.

## Density and spacing

- 4px grid (Tailwind's default spacing). Panes use `--pane-gutter` (12px compact, 16px
  comfortable) for horizontal padding.
- Thread rows use the `row-height` utility: 36px single-line in compact (default), 60px
  two-line in comfortable. The density switch is `data-density` on `<html>`, set from
  `src/lib/preferences.ts`; components read the token, not the preference.
- Radius `--radius` 8px; controls use `rounded-md` (6px), key caps `rounded-sm`.
- Lists are edge to edge with hairline separators, not cards. Cards are for settings
  groups and dialogs only.

## Layout

Three panes, resizable, sizes persisted per browser: sidebar (accounts, folders,
labels) | thread list | reading pane. `AppShell` owns the outer split and the 44px top bar;
`MailPanes` splits list and reader. The reading pane is never a modal on desktop widths.

## Motion

- Durations: `--duration-fast` 90ms (hover, press), `--duration-base` 160ms (menus,
  popovers, row insert), `--duration-slow` 240ms (sheets, dialogs).
- Easing: `ease-(--ease-out)` for entering, `ease-(--ease-in-out)` for moving.
- Motion confirms an action, it never delays one. Archive and move remove the row at once
  (optimistic) with a short height collapse; nothing waits for the engine.
- Empty states fade in (`animate-in fade-in`). No bouncing, no parallax.
- `prefers-reduced-motion` turns everything to 1ms (global rule in `index.css`).

## Iconography

lucide-react only, 16px (`size-4`) in toolbars and menus, 14px (`size-3.5`) inline with
`text-xs`. Stroke width default. Icons accompany text in menus; icon-only buttons need an
`aria-label` and a tooltip that names the shortcut.

## Keyboard

Every action reachable by mouse is reachable by key, and every shortcut is registered in
`src/lib/shortcuts.ts` so the `?` sheet lists it. Bindings follow Gmail/Superhuman habits:

| Keys                           | Action                      | Group       |
| ------------------------------ | --------------------------- | ----------- |
| `j` / `k` (also `down` / `up`) | next / previous thread      | Navigation  |
| `enter` or `o`                 | open thread                 | Navigation  |
| `escape` or `u`                | back to list                | Navigation  |
| `g i`                          | go to inbox                 | Navigation  |
| `g s` / `g d` / `g a`          | starred / drafts / all mail | Navigation  |
| `e`                            | archive                     | Thread      |
| `#`                            | trash                       | Thread      |
| `!`                            | spam                        | Thread      |
| `s`                            | star / unstar               | Thread      |
| `shift+u` / `shift+i`          | mark unread / read          | Thread      |
| `l` / `v`                      | label / move to             | Thread      |
| `x`                            | select row                  | Thread      |
| `r` / `a` / `f`                | reply / reply all / forward | Compose     |
| `c`                            | compose                     | Compose     |
| `mod+enter`                    | send (in composer)          | Compose     |
| `/`                            | search                      | Search      |
| `mod+k`                        | command palette             | Application |
| `?`                            | shortcuts sheet             | Application |

Rules:

- Single-key shortcuts never fire in inputs; `escape` and `mod+…` may (`allowInInputs`).
- Dialogs, the palette and the composer push a scope (`useShortcutScope`) so list keys
  stop while they are open.
- Show key caps with `<ShortcutKeys keys="g i" />` in tooltips, menus and empty states.
- Focus is always visible: `outline-ring` 2px with offset. The list's keyboard cursor is
  separate from browser focus and drawn with `row-focus`.

## Voice

Plain, short, specific. "Archived", "Sent", "Couldn't reach the engine". Empty states
get a short headline ("Quiet waters") and one line of help underneath.
No exclamation marks, no emoji. The product is "Tern"; never Mailspring.
