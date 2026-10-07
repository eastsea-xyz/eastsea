"""Render the EastSea app icon from the clean coin art.

macOS: the coin on a navy sea-gradient rounded square, on Apple's icon grid
(an 824 px body in a 1024 canvas, transparent corners, soft shadow).
iOS: the same art full-bleed and opaque (iOS applies its own mask).
Usage: python3 design/scripts/make-app-icon.py   (from the repo root)
"""
from PIL import Image, ImageDraw, ImageFilter

COIN = "design/assets/coin-1024.webp"
OUT = "apps/wallet/Assets.xcassets/AppIcon.appiconset"
MASTER = "design/assets/app-icon-1024.png"
TOP, BOTTOM = (18, 74, 122), (5, 22, 44)  # East Sea navy, light from above


def gradient(size):
    g = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / (size - 1)
        g.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    return g.resize((size, size))


def coin(diameter):
    c = Image.open(COIN).convert("RGBA")
    side = max(c.size)
    sq = Image.new("RGBA", (side, side))
    sq.paste(c, ((side - c.width) // 2, (side - c.height) // 2))
    # Trim the photo's dark fringe with a circular mask slightly inside the rim.
    mask = Image.new("L", (side, side))
    inset = side * 0.012
    ImageDraw.Draw(mask).ellipse((inset, inset, side - inset, side - inset), fill=255)
    sq.putalpha(Image.composite(sq.getchannel("A"), mask, mask))
    return sq.resize((diameter, diameter), Image.LANCZOS)


def with_coin(base, diameter, cx, cy, shadow_dy):
    c = coin(diameter)
    sh = Image.new("RGBA", base.size)
    blob = Image.new("L", base.size)
    r = diameter / 2
    ImageDraw.Draw(blob).ellipse((cx - r, cy - r + shadow_dy, cx + r, cy + r + shadow_dy), fill=150)
    sh.putalpha(blob.filter(ImageFilter.GaussianBlur(diameter * 0.04)))
    base = Image.alpha_composite(base, sh)
    base.alpha_composite(c, (round(cx - r), round(cy - r)))
    return base


def mac_master():
    canvas = Image.new("RGBA", (1024, 1024))
    body, off, radius = 824, 100, 185
    mask = Image.new("L", (1024, 1024))
    ImageDraw.Draw(mask).rounded_rectangle((off, off, off + body, off + body), radius=radius, fill=255)
    # The squircle's own drop shadow, per the macOS grid.
    shadow = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    sm = mask.filter(ImageFilter.GaussianBlur(14)).point(lambda v: v * 0.45)
    shadow.putalpha(sm)
    canvas = Image.alpha_composite(canvas, shadow.transform(canvas.size, Image.AFFINE, (1, 0, 0, 0, 1, -10)))
    bg = gradient(1024).convert("RGBA")
    bg.putalpha(mask)
    canvas = Image.alpha_composite(canvas, bg)
    return with_coin(canvas, 620, 512, 512, 14)


def ios_master():
    return with_coin(gradient(1024).convert("RGBA"), 720, 512, 512, 16).convert("RGB")


def main():
    mac = mac_master()
    mac.save(MASTER)
    for pt in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            mac.resize((pt * scale, pt * scale), Image.LANCZOS).save(f"{OUT}/mac-{pt}@{scale}x.png")
    ios_master().save(f"{OUT}/ios-1024.png")


if __name__ == "__main__":
    main()
