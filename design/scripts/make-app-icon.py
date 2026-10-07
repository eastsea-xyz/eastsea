"""Export the app and extension icons from Codex's brand masters (design/brand).

macOS slots 64 px and up come from app-icon-1024.png (macOS grid, transparent
corners); 16-32 px slots from the hand-simplified app-icon-32.png; iOS from the
opaque full-bleed app-icon-ios-1024.png.
Usage: python3 design/scripts/make-app-icon.py   (from the repo root)
"""
from PIL import Image

OUT = "apps/wallet/Assets.xcassets/AppIcon.appiconset"
big = Image.open("design/brand/app-icon-1024.png").convert("RGBA")
small = Image.open("design/brand/app-icon-32.png").convert("RGBA")


def render(px):
    src = small if px <= 32 else big
    return src.resize((px, px), Image.LANCZOS)


for pt in (16, 32, 128, 256, 512):
    for scale in (1, 2):
        render(pt * scale).save(f"{OUT}/mac-{pt}@{scale}x.png")
Image.open("design/brand/app-icon-ios-1024.png").convert("RGB").save(f"{OUT}/ios-1024.png")
for px in (16, 32, 48, 128):
    render(px).save(f"apps/extension/icons/{px}.png")
big.save("design/assets/app-icon-1024.png")
