"""Builds the application icon files from the artwork in crates/ketchup-app/assets.

Writes ketchup.ico (the Windows executable and installer icon) and
ketchup-128.rgba (the window icon: 128 x 128 straight RGBA, row-major).
The white background around the rounded square becomes transparent.
Needs Pillow; run it only when the artwork changes and commit the outputs.
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

ASSETS = Path(__file__).resolve().parent.parent / "crates" / "ketchup-app" / "assets"
SOURCE = ASSETS / "ketchup-icon.jpeg"
ICO_SIZES = [256, 128, 64, 48, 40, 32, 24, 20, 16]
WINDOW_SIZE = 128
# Background pixels are near white; the rounded square's blue rim stops the fill.
FILL_THRESHOLD = 60


def transparent_corners(image: Image.Image) -> Image.Image:
    marked = image.copy()
    width, height = marked.size
    key = (255, 0, 255)
    for seed in [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)]:
        ImageDraw.floodfill(marked, seed, key, thresh=FILL_THRESHOLD)
    red, green, blue = ImageChops.difference(marked, Image.new("RGB", marked.size, key)).split()
    mask = ImageChops.lighter(ImageChops.lighter(red, green), blue).point(lambda v: 255 if v else 0)
    rgba = image.convert("RGBA")
    rgba.putalpha(mask)
    box = mask.getbbox()
    if box is None:
        raise SystemExit("icon artwork has no foreground")
    left, top, right, bottom = box
    side = max(right - left, bottom - top)
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(rgba.crop(box), ((side - (right - left)) // 2, (side - (bottom - top)) // 2))
    return square


def scaled(image: Image.Image, size: int) -> Image.Image:
    # Premultiplied resampling keeps the white background out of the rim.
    return image.convert("RGBa").resize((size, size), Image.Resampling.LANCZOS).convert("RGBA")


def main() -> None:
    icon = transparent_corners(Image.open(SOURCE).convert("RGB"))
    largest = scaled(icon, ICO_SIZES[0])
    largest.save(
        ASSETS / "ketchup.ico",
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
        append_images=[scaled(icon, size) for size in ICO_SIZES[1:]],
    )
    (ASSETS / f"ketchup-{WINDOW_SIZE}.rgba").write_bytes(scaled(icon, WINDOW_SIZE).tobytes())


if __name__ == "__main__":
    main()
