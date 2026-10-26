"""Builds the GitHub art in branding/github/: the social preview card, the download
button and the framed demo GIF.

usage: python branding/make_github_art.py <inter.ttf> [<raw demo clip> <start s> <seconds>]
The demo clip is a screen recording of the main screen (2880x1620); the GIF frames the
middle of it like a screenshot on the File Beaver green.
"""
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
OUT = HERE / "github"
OUT.mkdir(exist_ok=True)
INTER = sys.argv[1]

POND, POND_DEEP, MOSS, LIME, FROST, INK = (14, 59, 36), (7, 33, 20), (47, 107, 63), (166, 228, 58), (241, 246, 238), (19, 38, 27)


def inter(size, weight, opsz=32):
    f = ImageFont.truetype(INTER, size)
    f.set_variation_by_axes([opsz, weight])
    return f


def pond_backdrop(w, h, glow_at=(0.78, 0.22), grain=1.4):
    """Deep pond green with a soft lime glow and a little grain, so it never bands."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    x /= w
    y /= h
    base = np.array(POND_DEEP, np.float32) + (np.array(POND, np.float32) - POND_DEEP) * (1 - y[..., None] * 0.6)
    d = ((x - glow_at[0]) / 0.55) ** 2 + ((y - glow_at[1]) / 0.75) ** 2
    glow = np.exp(-d)[..., None]
    img = base * (1 - glow * 0.55) + np.array(MOSS, np.float32) * glow * 0.35 + np.array(LIME, np.float32) * glow * 0.2
    if grain:
        img += np.random.default_rng(3).normal(0, grain, img.shape)
    return Image.fromarray(np.clip(img, 0, 255).astype(np.uint8))


def logo(size):
    im = Image.open(HERE / "file-beaver-logo-clean.png").convert("RGBA")
    return im.resize((size, size), Image.LANCZOS)


def shadowed(layer, radius, opacity):
    """A soft shadow the shape of `layer`'s alpha, on a canvas with room for the blur.
    Returns the shadow and how far it extends past the layer on each side."""
    m = radius * 3
    a = Image.new("L", (layer.width + 2 * m, layer.height + 2 * m), 0)
    a.paste(layer.split()[3].point(lambda v: int(v * opacity)), (m, m))
    sh = Image.new("RGBA", a.size, (0, 0, 0, 0))
    sh.putalpha(a.filter(ImageFilter.GaussianBlur(radius)))
    return sh, m


# ---------- social preview (1280x640, GitHub's recommended size) ----------

def social():
    W, H = 1280, 640
    im = pond_backdrop(W, H, glow_at=(0.25, 0.45)).convert("RGBA")
    lg = logo(430)
    sh, m = shadowed(lg, 24, 0.55)
    im.alpha_composite(sh, (90 - m, 125 - m))
    im.alpha_composite(lg, (80, 100))
    d = ImageDraw.Draw(im)
    d.text((560, 236), "File Beaver", font=inter(96, 850), fill=FROST, anchor="ls")
    d.text((562, 318), "Drag a file onto the beaver.", font=inter(42, 600), fill=(200, 222, 196), anchor="ls")
    d.text((562, 372), "Get it done.", font=inter(42, 700), fill=LIME, anchor="ls")
    small = inter(26, 550, 14)
    x = 562
    for i, word in enumerate(["Free", "Open source", "Works offline", "Windows"]):
        if i:
            d.ellipse((x + 10, 448, x + 16, 454), fill=(120, 160, 120))
            x += 26
        d.text((x, 460), word, font=small, fill=(170, 196, 168), anchor="ls")
        x += d.textlength(word, font=small)
    im.convert("RGB").save(OUT / "social-preview.png", optimize=True)


# ---------- download button (2x for retina; shown at 260x56) ----------

def windows_glyph(d, x, y, s, fill):
    g = s * 0.08
    h = (s - g) / 2
    for dx in (0, h + g):
        for dy in (0, h + g):
            d.rectangle((x + dx, y + dy, x + dx + h, y + dy + h), fill=fill)


def button():
    S = 2
    w, h = 260 * S, 56 * S
    im = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, w - 1, h - 1), radius=h // 2, fill=LIME + (255,))
    f = inter(19 * S, 750)
    text = "Download for Windows"
    tw = d.textlength(text, font=f)
    gs = 17 * S
    total = gs + 11 * S + tw
    x0 = (w - total) / 2
    windows_glyph(d, x0, (h - gs) / 2, gs, INK)
    d.text((x0 + gs + 11 * S, h / 2 + 1), text, font=f, fill=INK, anchor="lm")
    im.save(OUT / "download-button.png", optimize=True)


# ---------- framed demo GIF ----------

def demo_gif(clip, start, seconds):
    # Middle of the screen around the beaver and the ring (never the desktop icons).
    cx0, cy0, cw, ch = 760, 110, 1980, 1114
    inner_w = 880
    inner_h = round(inner_w * ch / cw)
    pad = 44
    W, H = inner_w + 2 * pad, inner_h + 2 * pad
    radius = 16

    bg = pond_backdrop(W, H, glow_at=(0.5, 1.25), grain=0).convert("RGBA")
    mask = Image.new("L", (inner_w, inner_h), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, inner_w - 1, inner_h - 1), radius, fill=255)
    card = Image.new("RGBA", (inner_w, inner_h), (0, 0, 0, 255))
    card.putalpha(mask)
    sh, m = shadowed(card, 16, 0.5)
    bg.alpha_composite(sh, (pad - m, pad + 8 - m))
    bg.save(OUT / "_frame_bg.png")
    mask.save(OUT / "_frame_mask.png")

    fc = (
        f"[0:v]crop={cw}:{ch}:{cx0}:{cy0},fps=15,scale={inner_w}:{inner_h}:flags=lanczos,format=rgba[v];"
        f"[2:v]fps=15,format=gray[m];[v][m]alphamerge[r];"
        f"[1:v]fps=15[bg];[bg][r]overlay={pad}:{pad}:shortest=1,split[a][b];"
        "[a]palettegen=max_colors=200:stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle"
    )
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-ss", str(start), "-t", str(seconds), "-i", clip,
         "-loop", "1", "-t", str(seconds), "-i", str(OUT / "_frame_bg.png"),
         "-loop", "1", "-t", str(seconds), "-i", str(OUT / "_frame_mask.png"),
         "-filter_complex", fc, "-frames:v", str(int(seconds * 15)), str(OUT / "demo.gif")],
        check=True,
    )
    (OUT / "_frame_bg.png").unlink()
    (OUT / "_frame_mask.png").unlink()


social()
button()
if len(sys.argv) >= 5:
    demo_gif(sys.argv[2], float(sys.argv[3]), float(sys.argv[4]))
print("done:", ", ".join(p.name for p in sorted(OUT.iterdir())))
