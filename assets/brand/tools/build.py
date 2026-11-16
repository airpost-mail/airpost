#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Airpost contributors
"""Builds the Airpost brand assets in assets/brand from the 1024 px masters.

    python3 -I assets/brand/tools/build.py             # derived sets + preview
    python3 -I assets/brand/tools/build.py --masters   # also re-clean masters from source/

Needs Pillow. The macOS .icns step needs iconutil (macOS only); elsewhere it
is skipped with a warning. Every output directory this script owns is wiped
and rewritten, so it is safe to re-run.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

BRAND = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(BRAND, 'source')
MASTERS = os.path.join(BRAND, 'masters')

M_TILE = os.path.join(MASTERS, 'airpost-icon-tile-1024.png')
M_MACOS = os.path.join(MASTERS, 'airpost-icon-macos-1024.png')
M_BIRD = {
    'on-dark': os.path.join(MASTERS, 'airpost-bird-on-dark-1024.png'),
    'on-light': os.path.join(MASTERS, 'airpost-bird-on-light-1024.png'),
}
M_SILHOUETTE = os.path.join(MASTERS, 'airpost-silhouette-1024.png')

CANVAS = 1024
# Apple's macOS icon grid: an 824 px tile centred on the 1024 canvas.
MACOS_TILE = 824
# Windows and Linux icons get a small margin (about 5 % per side).
DESKTOP_TILE = 928

# Alpha cleanup for the AI-exported birds. Alpha <= LO is fringe noise; the
# interior sits at 252-253 rather than 255. A linear ramp LO..HI -> 0..255
# drops the fringe without a step at the threshold and makes the body opaque.
ALPHA_LO = 16
ALPHA_HI = 250

LINUX_SIZES = [16, 32, 64, 128, 256, 512]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
WEB_SIZES = [64, 128, 256, 512]
ICONSET = [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)]
# Tray .ico frames for 100..400 % scaling, as in the existing system-tray build-win32-ico.py.
TRAY_ICO_SIZES = [16, 20, 24, 28, 32, 40, 48, 64]

# Tray states, mirroring src/features/system-tray/assets (MenuItem-Inbox-*).
# Colours are the ones the existing tray icons use.
GREY, WHITE = (52, 57, 64), (255, 255, 255)
TRAY_STATES = {
    # name: (light-panel colour, dark-panel colour, alpha factor)
    'airpost-tray-full': (GREY, WHITE, 1.0),
    # Inbox zero: same bird, dimmed. A placeholder until a designer draws a real empty state.
    'airpost-tray-zero': (GREY, WHITE, 0.5),
    'airpost-tray-full-new': ((246, 61, 96), (246, 61, 141), 1.0),
    'airpost-tray-full-unread': ((9, 110, 230), (54, 197, 240), 1.0),
}
# Which states are macOS template images (black + alpha, recoloured by the menu bar).
DARWIN_TEMPLATES = {'airpost-tray-full', 'airpost-tray-zero'}


def log(msg):
    sys.stdout.write(msg + '\n')


def reset_dir(path):
    if os.path.isdir(path):
        shutil.rmtree(path)
    os.makedirs(path)


def save_png(img, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.save(path, optimize=True)


def resize(img, size):
    # Pillow premultiplies RGBA internally for non-nearest filters, so edges don't darken.
    if img.size == size:
        return img.copy()
    return img.resize(size, Image.LANCZOS)


def pad(img, inner, canvas=CANVAS):
    out = Image.new('RGBA', (canvas, canvas), (0, 0, 0, 0))
    tile = resize(img, (inner, inner))
    off = (canvas - inner) // 2
    out.alpha_composite(tile, (off, off))
    return out


def zero_transparent_rgb(img):
    # Fully transparent pixels keep whatever RGB the exporter left; clear it so
    # the PNGs compress well and nothing leaks through a careless un-premultiply.
    a = img.getchannel('A')
    clear = Image.new('RGBA', img.size, (0, 0, 0, 0))
    return Image.composite(img, clear, a.point(lambda v: 255 if v else 0))


# ---------------------------------------------------------------- masters


def clean_bird(src):
    img = Image.open(src).convert('RGBA')
    a = img.getchannel('A')
    span = ALPHA_HI - ALPHA_LO
    lut = [0 if v <= ALPHA_LO else min(255, round((v - ALPHA_LO) * 255 / span)) for v in range(256)]
    a = a.point(lut)

    # Keep only the bird's connected region, in case a speck survives the threshold.
    mask = a.point(lambda v: 255 if v else 0)
    px = a.load()
    seed = next(((x, y) for y in range(0, CANVAS, 8) for x in range(0, CANVAS, 8) if px[x, y] == 255), None)
    if seed is None:
        raise SystemExit(f'{src}: no opaque pixel found')
    ImageDraw.floodfill(mask, seed, 128)
    hist = mask.histogram()
    if hist[255]:
        log(f'  {os.path.basename(src)}: dropped {hist[255]} px outside the main shape')
    a = ImageChops.multiply(a, mask.point(lambda v: 255 if v == 128 else 0))

    img.putalpha(a)
    img = zero_transparent_rgb(img)

    # Centre the bounding box. Integer shift only, so the master isn't resampled.
    x0, y0, x1, y1 = a.getbbox()
    dx = (CANVAS - (x1 - x0)) // 2 - x0
    dy = (CANVAS - (y1 - y0)) // 2 - y0
    out = Image.new('RGBA', img.size, (0, 0, 0, 0))
    out.paste(img, (dx, dy))
    bb = out.getchannel('A').getbbox()
    log(f'  {os.path.basename(src)}: bbox {bb}, margins L{bb[0]} R{CANVAS - bb[2]} T{bb[1]} B{CANVAS - bb[3]}')
    return out


def build_masters():
    log('masters/')
    reset_dir(MASTERS)
    tile = zero_transparent_rgb(Image.open(os.path.join(SOURCE, 'airpost-dark-icon-1024.png')).convert('RGBA'))
    save_png(tile, M_TILE)
    save_png(pad(tile, MACOS_TILE), M_MACOS)
    birds = {
        'on-dark': 'airpost-bird-for-dark-background-1024.png',
        'on-light': 'airpost-bird-for-light-background-1024.png',
    }
    for key, name in birds.items():
        save_png(clean_bird(os.path.join(SOURCE, name)), M_BIRD[key])
    # Monochrome silhouette (black + alpha) for tray and single-colour uses.
    a = Image.open(M_BIRD['on-light']).getchannel('A')
    sil = Image.new('RGBA', a.size, (0, 0, 0, 0))
    sil.putalpha(a)
    save_png(zero_transparent_rgb(sil), M_SILHOUETTE)


# ---------------------------------------------------------------- derived sets


def build_macos(macos_icon, out_dir):
    reset_dir(out_dir)
    if not shutil.which('iconutil'):
        log('  warning: iconutil not found, skipping airpost.icns')
        return
    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, 'airpost.iconset')
        os.makedirs(iconset)
        for pt, scale in ICONSET:
            px = pt * scale
            suffix = '' if scale == 1 else '@2x'
            resize(macos_icon, (px, px)).save(os.path.join(iconset, f'icon_{pt}x{pt}{suffix}.png'))
        subprocess.run(['iconutil', '-c', 'icns', iconset, '-o', os.path.join(out_dir, 'airpost.icns')], check=True)


def save_ico(frames, path, bitmap_format=None):
    # Pillow writes the frames it is given via append_images; the base must be the largest.
    frames = sorted(frames, key=lambda f: f.size[0], reverse=True)
    kwargs = {'sizes': [f.size for f in frames], 'append_images': frames[1:]}
    if bitmap_format:
        kwargs['bitmap_format'] = bitmap_format
    frames[0].save(path, format='ICO', **kwargs)


def build_windows(desktop_icon, tile, out_dir):
    reset_dir(out_dir)
    save_ico([resize(desktop_icon, (s, s)) for s in ICO_SIZES], os.path.join(out_dir, 'airpost.ico'))
    # Start-menu tiles (VisualElementsManifest); the old ones are full bleed.
    for s in (150, 75):
        save_png(resize(tile, (s, s)), os.path.join(out_dir, f'airpost-{s}px.png'))


def build_linux(desktop_icon, out_dir):
    reset_dir(out_dir)
    for s in LINUX_SIZES:
        save_png(resize(desktop_icon, (s, s)), os.path.join(out_dir, f'airpost-{s}.png'))


def build_web(out_dir):
    reset_dir(out_dir)
    for key, path in M_BIRD.items():
        bird = Image.open(path).convert('RGBA')
        for s in WEB_SIZES:
            save_png(resize(bird, (s, s)), os.path.join(out_dir, f'airpost-bird-{key}-{s}.png'))


class TrayRenderer:
    """Renders the bird silhouette into small square tray canvases."""

    def __init__(self, silhouette):
        a = silhouette.getchannel('A')
        self.mask = a.crop(a.getbbox())
        self._dilated = {}

    def _source(self, size):
        # At 16-32 px the tail streamers are under a pixel wide and vanish.
        # Thicken the shape by about a quarter of an output pixel before
        # downsampling, a bit less at 40-64 px, none above.
        w, h = self.mask.size
        k = (size - max(1, size // 16)) / max(w, h)
        grow = 0.25 if size <= 32 else (0.15 if size <= 64 else 0)
        radius = int(round(grow / k))
        if radius < 1:
            return self.mask
        if radius not in self._dilated:
            self._dilated[radius] = self.mask.filter(ImageFilter.MaxFilter(2 * radius + 1))
        return self._dilated[radius]

    def alpha(self, size):
        src = self._source(size)
        w, h = src.size
        box = size - max(1, size // 16)
        k = box / max(w, h)
        r = src.resize((max(1, round(w * k)), max(1, round(h * k))), Image.LANCZOS)
        out = Image.new('L', (size, size), 0)
        out.paste(r, ((size - r.size[0]) // 2, (size - r.size[1]) // 2))
        return out

    def render(self, size, colour, factor=1.0):
        a = self.alpha(size)
        if factor != 1.0:
            a = a.point(lambda v: round(v * factor))
        img = Image.new('RGBA', (size, size), colour + (0,))
        img.putalpha(a)
        return zero_transparent_rgb(img)


def build_tray(silhouette, out_dir):
    reset_dir(out_dir)
    tr = TrayRenderer(silhouette)
    for name, (light, dark, factor) in TRAY_STATES.items():
        # macOS: 16 / 32 px. Templates are pure black + alpha.
        if name in DARWIN_TEMPLATES:
            base, colour = f'{name}-Template', (0, 0, 0)
        else:
            base, colour = name, light
        save_png(tr.render(16, colour, factor), os.path.join(out_dir, 'darwin', f'{base}.png'))
        save_png(tr.render(32, colour, factor), os.path.join(out_dir, 'darwin', f'{base}@2x.png'))
        for variant, col in (('', light), ('-dark', dark)):
            # Linux: 32 px only.
            save_png(tr.render(32, col, factor), os.path.join(out_dir, 'linux', f'{name}{variant}.png'))
            # Windows: 16 / 32 px PNGs plus a multi-size .ico (BMP frames, see the
            # existing system-tray/assets/build-win32-ico.py for why).
            win = os.path.join(out_dir, 'win32')
            save_png(tr.render(16, col, factor), os.path.join(win, f'{name}{variant}.png'))
            save_png(tr.render(32, col, factor), os.path.join(win, f'{name}{variant}@2x.png'))
            save_ico([tr.render(s, col, factor) for s in TRAY_ICO_SIZES], os.path.join(win, f'{name}{variant}.ico'), 'bmp')


# ---------------------------------------------------------------- preview

LIGHT_BG, DARK_BG = (245, 245, 247), (30, 30, 32)
LIGHT_FG, DARK_FG = (40, 40, 40), (225, 225, 225)
GAP = 16


def font(size):
    try:
        return ImageFont.load_default(size=size)
    except TypeError:  # Pillow < 10.1
        return ImageFont.load_default()


def ico_frames(path):
    im = Image.open(path)
    frames = []
    for s in sorted(im.info['sizes']):
        im.size = s
        im.load()
        frames.append(im.convert('RGBA').copy())
    return frames


def template_on(img, dark):
    # How macOS draws a template image: black on a light menu bar, white on a dark one.
    a = img.getchannel('A')
    out = Image.new('RGBA', img.size, (255, 255, 255, 0) if dark else (0, 0, 0, 0))
    out.putalpha(a)
    return out


def zoom(img, factor):
    return img.resize((img.size[0] * factor, img.size[1] * factor), Image.NEAREST)


def preview_sections():
    """Returns [(title, [(label, light_img, dark_img)])]. Same image for both unless it adapts."""
    sec = []

    def same(label, img):
        return (label, img, img)

    with tempfile.TemporaryDirectory() as tmp:
        icns = os.path.join(BRAND, 'macos', 'airpost.icns')
        items = []
        if os.path.exists(icns):
            out = os.path.join(tmp, 'x.iconset')
            subprocess.run(['iconutil', '-c', 'iconset', icns, '-o', out], check=True)
            for pt, scale in ICONSET:
                suffix = '' if scale == 1 else '@2x'
                img = Image.open(os.path.join(out, f'icon_{pt}x{pt}{suffix}.png')).convert('RGBA')
                items.append(same(f'{pt}{suffix}', img))
        sec.append(('macOS airpost.icns (every iconset entry, actual pixels)', items))

    sec.append(('Windows airpost.ico frames', [same(str(f.size[0]), f) for f in ico_frames(os.path.join(BRAND, 'windows', 'airpost.ico'))]
                + [same(f'tile {s}', Image.open(os.path.join(BRAND, 'windows', f'airpost-{s}px.png')).convert('RGBA')) for s in (75, 150)]))
    sec.append(('Linux airpost-<size>.png', [same(str(s), Image.open(os.path.join(BRAND, 'linux', f'airpost-{s}.png')).convert('RGBA')) for s in LINUX_SIZES]))

    for key in ('on-light', 'on-dark'):
        sec.append((f'Web marks airpost-bird-{key}', [same(str(s), Image.open(os.path.join(BRAND, 'web', f'airpost-bird-{key}-{s}.png')).convert('RGBA')) for s in WEB_SIZES]))

    darwin = os.path.join(BRAND, 'tray', 'darwin')
    items = []
    for name in TRAY_STATES:
        base = f'{name}-Template' if name in DARWIN_TEMPLATES else name
        for suffix in ('', '@2x'):
            img = Image.open(os.path.join(darwin, f'{base}{suffix}.png')).convert('RGBA')
            if name in DARWIN_TEMPLATES:
                items.append((f'{name[10:]}{suffix}', template_on(img, False), template_on(img, True)))
            else:
                items.append(same(f'{name[10:]}{suffix}', img))
    sec.append(('macOS menu bar (templates drawn as macOS would: black on light, white on dark)', items))

    items = []
    for name in TRAY_STATES:
        for suffix in ('', '@2x'):
            li = Image.open(os.path.join(BRAND, 'tray', 'win32', f'{name}{suffix}.png')).convert('RGBA')
            di = Image.open(os.path.join(BRAND, 'tray', 'win32', f'{name}-dark{suffix}.png')).convert('RGBA')
            items.append((f'{name[10:]}{suffix}', li, di))
    sec.append(('Windows / Linux tray (plain on light, -dark on dark; Linux uses the 32 px)', items))

    items = []
    tray16 = template_on(Image.open(os.path.join(darwin, 'airpost-tray-full-Template.png')).convert('RGBA'), False)
    tray16d = template_on(Image.open(os.path.join(darwin, 'airpost-tray-full-Template.png')).convert('RGBA'), True)
    tray32 = template_on(Image.open(os.path.join(darwin, 'airpost-tray-full-Template@2x.png')).convert('RGBA'), False)
    tray32d = template_on(Image.open(os.path.join(darwin, 'airpost-tray-full-Template@2x.png')).convert('RGBA'), True)
    lin = {s: Image.open(os.path.join(BRAND, 'linux', f'airpost-{s}.png')).convert('RGBA') for s in (16, 32)}
    items.append(('icon 16 x8', zoom(lin[16], 8), zoom(lin[16], 8)))
    items.append(('icon 32 x4', zoom(lin[32], 4), zoom(lin[32], 4)))
    items.append(('tray 16 x8', zoom(tray16, 8), zoom(tray16d, 8)))
    items.append(('tray 32 x4', zoom(tray32, 4), zoom(tray32d, 4)))
    sec.append(('Zoomed small sizes (nearest neighbour)', items))
    return sec


def item_w(item, label_f):
    label, img, _ = item
    return max(img.size[0], int(label_f.getlength(label)) + 4)


def build_preview(path):
    sections = preview_sections()
    title_f, label_f = font(16), font(11)
    # Lay each section out in lines no wider than max_w; each item is at least as wide as its label.
    max_w = max(1400, max(it[1].size[0] for _t, items in sections for it in items) + 2 * GAP)
    rows = []
    for title, items in sections:
        lines, line, w = [], [], GAP
        for it in items:
            iw = item_w(it, label_f) + GAP
            if line and w + iw > max_w:
                lines.append(line)
                line, w = [], GAP
            line.append(it)
            w += iw
        lines.append(line)
        rows.append((title, [(ln, max(it[1].size[1] for it in ln)) for ln in lines]))
    total_h = GAP + sum(26 + sum(h + 34 for _ln, h in lines) for _t, lines in rows)
    sheet = Image.new('RGBA', (max_w * 2, total_h), (0, 0, 0, 255))
    for side, (bg, fg) in enumerate(((LIGHT_BG, LIGHT_FG), (DARK_BG, DARK_FG))):
        panel = Image.new('RGBA', (max_w, total_h), bg + (255,))
        d = ImageDraw.Draw(panel)
        y = GAP
        for title, lines in rows:
            d.text((GAP, y), title, fill=fg, font=title_f)
            y += 26
            for line, h in lines:
                x = GAP
                for label, li, di in line:
                    img = li if side == 0 else di
                    panel.alpha_composite(img, (x, y + h - img.size[1]))
                    d.text((x, y + h + 4), label, fill=fg, font=label_f)
                    x += item_w((label, li, di), label_f) + GAP
                y += h + 34
        sheet.paste(panel, (side * max_w, 0))
    sheet.convert('RGB').save(path, optimize=True)


# ---------------------------------------------------------------- main


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--masters', action='store_true', help='re-clean masters/ from source/ first')
    args = ap.parse_args()

    if args.masters:
        build_masters()

    tile = Image.open(M_TILE).convert('RGBA')
    macos_icon = Image.open(M_MACOS).convert('RGBA')
    desktop_icon = pad(tile, DESKTOP_TILE)
    silhouette = Image.open(M_SILHOUETTE).convert('RGBA')

    log('macos/')
    build_macos(macos_icon, os.path.join(BRAND, 'macos'))
    log('windows/')
    build_windows(desktop_icon, tile, os.path.join(BRAND, 'windows'))
    log('linux/')
    build_linux(desktop_icon, os.path.join(BRAND, 'linux'))
    log('web/')
    build_web(os.path.join(BRAND, 'web'))
    log('tray/')
    build_tray(silhouette, os.path.join(BRAND, 'tray'))
    log('preview.png')
    build_preview(os.path.join(BRAND, 'preview.png'))


if __name__ == '__main__':
    main()
