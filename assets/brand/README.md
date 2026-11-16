# Airpost brand assets

App icon, mail cube marks and tray icons for Airpost. The table at the end says where each file is used in `apps/desktop`.

## Origin

The mark is a mail cube: a box whose lid and sides fold like an envelope. It is a vector recreation made with ChatGPT from its "B11 Mail Cube" concept. The vectors are kept in `source/`: `airpost-mail-cube-{original,blue,black,white}.svg` (the mark alone) and `airpost-cube-icon.svg` (the full app-icon recreation).

## Licence

The logos are trademarks, all rights reserved, and not covered by the GPL; forks must replace them. See `LICENSE.md` here. `tools/build.py` is code and is GPL-3.0-only like the rest of the repo.

## Files

| Path | What it is |
| --- | --- |
| `source/airpost-mail-cube-*.svg`, `source/airpost-cube-icon.svg` | The vector sources. |
| `masters/airpost-icon-tile-1024.png` | The app icon: the original mark at 64 % on a square rounded tile with the blue gradient from `airpost-cube-icon.svg` (`#27C3F6` to `#0730C7`). Base for Windows and Linux. |
| `masters/airpost-icon-macos-1024.png` | The same tile at 824x824, centred on a transparent 1024 canvas (macOS icon grid). |
| `masters/airpost-bird-on-dark-1024.png` | The original (white and light blue) mark, for dark backgrounds. |
| `masters/airpost-bird-on-light-1024.png` | The blue mark, for light backgrounds. |
| `masters/airpost-silhouette-1024.png` | The black mark, with transparent gaps between the faces. Base for the tray icons. |
| `macos/airpost.icns` | macOS app icon, full iconset 16..512 @1x/@2x, built with `iconutil`. |
| `windows/airpost.ico` | Windows app icon, frames 16, 24, 32, 48, 64, 128, 256 (PNG-compressed). |
| `windows/airpost-150px.png`, `airpost-75px.png` | Start-menu tiles for the VisualElements manifest. |
| `linux/airpost-{16,32,64,128,256,512}.png` | Linux hicolor icons, same size set as `apps/desktop/build/resources/linux/icons`. |
| `tray/darwin/` | Menu bar icons, 16 px and @2x 32 px. `-Template` files are black + alpha macOS template images. |
| `tray/linux/` | 32 px tray icons, plain (for light panels) and `-dark` (white, for dark panels). |
| `tray/win32/` | 16 px and @2x PNGs plus multi-size `.ico` (16..64, BMP frames) for every state, plain and `-dark`. |
| `web/airpost-bird-{on-dark,on-light}-{64,128,256,512}.png` | Marks for web and in-app use. The files keep their `bird` names so the code that loads them is unchanged. |
| `preview.png` | Contact sheet of every derived size on a light and a dark background, with 16/32 px zoomed. |
| `tools/build.py` | Rebuilds everything above from `masters/`. |

The masters are exported from the vectors and committed as they are. `build.py --masters` cleaned the previous bird artwork and doesn't apply to them. Windows and Linux icons use the tile scaled to 928/1024 (about a 5 % margin per side); macOS uses the 824 grid master.

### Tray states

These mirror the four states in `apps/desktop/src/features/system-tray` (see `system-tray-icon-store.ts`) and reuse the old colours from its `assets/build-win32-ico.py`.

| Old name | New name | Look |
| --- | --- | --- |
| `MenuItem-Inbox-Full` | `airpost-tray-full` | Cube silhouette; grey `#343940` (white for `-dark`), black template on macOS. |
| `MenuItem-Inbox-Zero` | `airpost-tray-zero` | Same silhouette at 50 % alpha. Placeholder for a proper "empty" mark. |
| `MenuItem-Inbox-Full-NewItems` | `airpost-tray-full-new` | Red `#F63D60` (`-dark`: `#F63D8D`). Not a template on macOS. |
| `MenuItem-Inbox-Full-UnreadItems` | `airpost-tray-full-unread` | Blue `#096EE6` (`-dark`: `#36C5F0`). Not a template on macOS. |

At 32 px and below the silhouette is thickened by about a quarter of an output pixel before downsampling (less at 40-64 px).

## Rebuilding

From the repo root, with Pillow installed (macOS needed for `iconutil`; elsewhere the `.icns` is skipped with a warning):

```sh
python3 -I assets/brand/tools/build.py             # derived sets + preview.png from masters/
```

The script wipes and rewrites `masters/` (with `--masters`), `macos/`, `windows/`, `linux/`, `tray/`, `web/` and `preview.png`. Output is byte-for-byte reproducible on the same Pillow version.

## Where each asset goes in apps/desktop

Paths relative to `apps/desktop`.

| Asset | Copy | Used by |
| --- | --- | --- |
| `macos/airpost.icns` | `build/resources/mac/airpost.icns` | `build/build.js` (packager `icon.darwin`). Also what the macOS About panel shows (`application:about` calls the standard About panel). |
| `windows/airpost.ico` | `build/resources/win/airpost.ico` | `build/build.js` (`icon.win32`), `build/create-signed-windows-installer.js` (`setupIcon`, and `iconUrl` points at this file on GitHub), `src/main/autoupdate/win32-updater.js` (shortcut icon). |
| `windows/airpost-150px.png`, `airpost-75px.png` | `build/resources/win/airpost-150px.png`, `airpost-75px.png` | `airpost.VisualElementsManifest.xml` (background `#0589EE`). |
| `linux/airpost-<n>.png` | `build/resources/linux/icons/airpost-<n>.png` | Installed as `hicolor/<n>x<n>/apps/com.airpost.airpost.png` by `build/resources/linux/redhat/airpost.spec.in` and `build/mkdeb`; `airpost-512.png` also goes to `/usr/share/pixmaps/airpost.png`. |
| `linux/airpost-512.png` | `static/images/airpost.png` | Window icon on Linux (`src/main/windows/app-window.ts`), native notifications (`src/window/platform/native-notifications.ts`), update notification (`src/main/autoupdate/manager.ts`). |
| `tray/darwin/*`, `tray/linux/*`, `tray/win32/*` | `src/features/system-tray/assets/{darwin,linux,win32}/` | `src/features/system-tray/icon-store.ts` and `_defaultIconPath` in `src/main/platform/system-tray.ts`. |
| `web/airpost-bird-on-dark-256.png` | `src/features/onboarding/assets/airpost-bird-on-dark-256.png` | Mark above the title on the onboarding welcome screen (`page-welcome.tsx`); the generic icon pattern `icons-bg@2x.png` stays as its background. |
| `web/airpost-bird-*` | `apps/web/public/brand/` (repo root) | The web client's logo (`apps/web/src/components/Logo.tsx`). `apps/web/public/favicon.png` is the app icon tile at 64 px. |

Not covered here: a document icon, a macOS 26 Icon Composer icon (`airpost.icon`, needs layered vector artwork; without it macOS 26 shows `airpost.icns`) and `build/resources/win/loading.gif` (the Windows installer animation).
