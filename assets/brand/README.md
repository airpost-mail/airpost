# Tern brand assets

App icon, bird marks and tray icons for Tern. Nothing in `apps/desktop` uses these yet; the table at the end says where each file goes during the rebrand.

## Origin

The artwork was AI-generated with an image generation/editing tool from the user's "Origami Bird App Icon" (the app icon tile and two standalone bird exports). `ternmail-logo-notes.txt` is the note that came with the exports, including the prompts used. The 1024 px originals are kept unmodified in `source/`.

## Licence

Licence: not decided (logos are usually excluded from the GPL as trademarks). `tools/build.py` is code and is GPL-3.0-only like the rest of the repo.

## Files

| Path | What it is |
| --- | --- |
| `source/ternmail-dark-icon-1024.png` | Original app icon: navy rounded-square tile, full bleed on the 1024 canvas. |
| `source/ternmail-bird-for-{dark,light}-background-1024.png` | Original standalone birds on transparency. |
| `masters/tern-icon-tile-1024.png` | The app icon tile, full bleed (source with transparent pixels' RGB zeroed). Base for Windows and Linux. |
| `masters/tern-icon-macos-1024.png` | The tile scaled to 824x824 and centred on a transparent 1024 canvas (macOS icon grid). Corners scaled with it, no shadow added. |
| `masters/tern-bird-on-dark-1024.png` | White and blue bird, for dark backgrounds. Cleaned and centred. |
| `masters/tern-bird-on-light-1024.png` | Navy and cobalt bird, for light backgrounds. Cleaned and centred. |
| `masters/tern-silhouette-1024.png` | Black + alpha silhouette taken from the on-light bird. Base for the tray icons. |
| `macos/tern.icns` | macOS app icon, full iconset 16..512 @1x/@2x, built with `iconutil`. |
| `windows/tern.ico` | Windows app icon, frames 16, 24, 32, 48, 64, 128, 256 (PNG-compressed). |
| `windows/tern-150px.png`, `tern-75px.png` | Start-menu tiles for the VisualElements manifest (full bleed, like the old ones). |
| `linux/tern-{16,32,64,128,256,512}.png` | Linux hicolor icons, same size set as `apps/desktop/app/build/resources/linux/icons`. |
| `tray/darwin/` | Menu bar icons, 16 px and @2x 32 px. `-Template` files are black + alpha macOS template images. |
| `tray/linux/` | 32 px tray icons, plain (for light panels) and `-dark` (white, for dark panels). |
| `tray/win32/` | 16 px and @2x PNGs plus multi-size `.ico` (16..64, BMP frames) for every state, plain and `-dark`. |
| `web/tern-bird-{on-dark,on-light}-{64,128,256,512}.png` | Bird marks for web and in-app use. |
| `preview.png` | Contact sheet of every derived size on a light and a dark background, with 16/32 px zoomed. |
| `tools/build.py` | Rebuilds everything above. |

### Master cleanup

The bird exports had faint noise (alpha up to 16) scattered over the canvas, reaching the edges on the light-background bird, and the bird's interior sat at alpha 252-253 rather than 255. `build.py --masters` maps alpha linearly from 16..250 to 0..255 (no step at the threshold, so the anti-aliasing stays smooth), keeps only the bird's connected region, zeroes RGB under transparent pixels and centres the bounding box with an integer shift (no resampling). Resulting margins: on-dark L30 R31 T102 B102, on-light L26 R26 T116 B116.

The app icon tile needed no cleanup beyond zeroing RGB under transparency. Windows and Linux icons use the tile scaled to 928/1024 (about a 5 % margin per side, close to the old Mailspring icons); macOS uses the 824 grid master.

### Tray states

These mirror the four states in `apps/desktop/app/internal_packages/system-tray` (see `lib/system-tray-icon-store.ts`) and reuse the old colours from its `assets/build-win32-ico.py`.

| Old name | New name | Look |
| --- | --- | --- |
| `MenuItem-Inbox-Full` | `tern-tray-full` | Bird silhouette; grey `#343940` (white for `-dark`), black template on macOS. |
| `MenuItem-Inbox-Zero` | `tern-tray-zero` | Same silhouette at 50 % alpha. Placeholder for a proper "empty" mark. |
| `MenuItem-Inbox-Full-NewItems` | `tern-tray-full-new` | Red `#F63D60` (`-dark`: `#F63D8D`). Not a template on macOS. |
| `MenuItem-Inbox-Full-UnreadItems` | `tern-tray-full-unread` | Blue `#096EE6` (`-dark`: `#36C5F0`). Not a template on macOS. |

At 32 px and below the silhouette is thickened by about a quarter of an output pixel before downsampling (less at 40-64 px) so the tail streamers don't vanish.

## Rebuilding

From the repo root, with Pillow installed (macOS needed for `iconutil`; elsewhere the `.icns` is skipped with a warning):

```sh
python3 -I assets/brand/tools/build.py             # derived sets + preview.png from masters/
python3 -I assets/brand/tools/build.py --masters   # re-clean masters/ from source/ first
```

The script wipes and rewrites `masters/` (with `--masters`), `macos/`, `windows/`, `linux/`, `tray/`, `web/` and `preview.png`. Output is byte-for-byte reproducible on the same Pillow version.

## Where each asset goes in apps/desktop

Paths relative to `apps/desktop/app`. Not done yet; this is the plan for the rebrand.

| Asset | Replaces | Used by |
| --- | --- | --- |
| `macos/tern.icns` | `build/resources/mac/mailspring.icns` | `build/build.js` (packager `icon.darwin`). Also what the macOS About panel shows (`application:about` calls the standard About panel). |
| `windows/tern.ico` | `build/resources/win/mailspring-square.ico` and `mailspring.ico` | `build/build.js` (`icon.win32`), `build/create-signed-windows-installer.js` (`setupIcon`; its `iconUrl` still points at a Mailspring S3 bucket), `src/browser/windows-updater.js` (shortcut icon). |
| `windows/tern-150px.png`, `tern-75px.png` | `build/resources/win/mailspring-150px.png`, `mailspring-75px.png` | `mailspring.VisualElementsManifest.xml`; its `BackgroundColor='#4ecdc4'` should become the tile navy, about `#0A1A3D`. |
| `linux/tern-<n>.png` | `build/resources/linux/icons/<n>.png` | Copied to `hicolor/<n>x<n>/apps/` by `build/resources/linux/redhat/mailspring.spec.in`; the deb (`build/build.js`, `script/mkdeb`) takes only `512.png`. |
| `linux/tern-512.png` | `static/images/mailspring.png` | Window icon on Linux (`src/browser/mailspring-window.ts`), native notifications (`src/native-notifications.ts`), update notification (`src/browser/autoupdate-manager.ts`). |
| `tray/darwin/*`, `tray/linux/*`, `tray/win32/*` | `internal_packages/system-tray/assets/{darwin,linux,win32}/MenuItem-Inbox-*` | `internal_packages/system-tray/lib/system-tray-icon-store.ts` and `_defaultIconPath` in `src/browser/system-tray-manager.ts` (file names change). `assets/build-win32-ico.py` and the `.psd` files become obsolete. |
| `web/tern-bird-*` | `internal_packages/onboarding/assets/icons-bg@2x.png` (welcome page, `lib/page-welcome.tsx`) | Onboarding welcome screen; also any About/preferences or website use of the mark. |

Not covered here: `build/resources/mac/file.icns` (document icon), `build/resources/mac/mailspring.icon` (macOS 26 Icon Composer icon, needs layered vector artwork), `build/resources/mac/DMG-Background.png`, and `build/resources/win/loading.gif`.
