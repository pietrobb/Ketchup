"""Builds the application icon files from the artwork in crates/ketchup-app/assets.

Writes ketchup.ico (the Windows executable and installer icon) and
ketchup-128.rgba (the window icon: 128 x 128 straight RGBA, row-major).
The Viewer gets the same artwork on a dark green background with a large eye
over the lower half, so it differs from the editor even at 16 px,
as ketchup-viewer.ico and ketchup-viewer-128.rgba in crates/ketchup-viewer/assets.
The white background around the rounded square becomes transparent.
Needs Pillow; run it only when the artwork changes and commit the outputs.
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

CRATES = Path(__file__).resolve().parent.parent / "crates"
ASSETS = CRATES / "ketchup-app" / "assets"
VIEWER_ASSETS = CRATES / "ketchup-viewer" / "assets"
SOURCE = ASSETS / "ketchup-icon.jpeg"
ICO_SIZES = [256, 128, 64, 48, 40, 32, 24, 20, 16]
WINDOW_SIZE = 128
# Background pixels are near white; the rounded square's blue rim stops the fill.
FILL_THRESHOLD = 60
NAVY = (20, 40, 90, 255)
WHITE = (255, 255, 255, 255)
IRIS = (30, 110, 220, 255)
PUPIL = (10, 20, 40, 255)


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


def write_icons(icon: Image.Image, folder: Path, stem: str) -> None:
    largest = scaled(icon, ICO_SIZES[0])
    largest.save(
        folder / f"{stem}.ico",
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
        append_images=[scaled(icon, size) for size in ICO_SIZES[1:]],
    )
    (folder / f"{stem}-{WINDOW_SIZE}.rgba").write_bytes(scaled(icon, WINDOW_SIZE).tobytes())


def almond(side: int, cx: float, cy: float, width: float, height: float) -> Image.Image:
    radius = (width * width / 4 + height * height / 4) / height
    upper = Image.new("L", (side, side), 0)
    ImageDraw.Draw(upper).ellipse([cx - radius, cy - height / 2, cx + radius, cy - height / 2 + 2 * radius], fill=255)
    lower = Image.new("L", (side, side), 0)
    ImageDraw.Draw(lower).ellipse([cx - radius, cy + height / 2 - 2 * radius, cx + radius, cy + height / 2], fill=255)
    return ImageChops.multiply(upper, lower)


def big_eye(side: int, cx: float, cy: float, width: float, height: float, rim: float, iris_color) -> Image.Image:
    """A large almond eye drawn on a transparent layer of the given side."""
    layer = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    layer.paste(Image.new("RGBA", (side, side), NAVY), (0, 0), almond(side, cx, cy, width, height))
    inner = almond(side, cx, cy, width - 2 * rim, height - 2 * rim)
    layer.paste(Image.new("RGBA", (side, side), WHITE), (0, 0), inner)
    def dot(r):
        mask = Image.new("L", (side, side), 0)
        ImageDraw.Draw(mask).ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
        return ImageChops.multiply(mask, inner)

    iris = height * 0.36
    layer.paste(Image.new("RGBA", (side, side), iris_color), (0, 0), dot(iris))
    layer.paste(Image.new("RGBA", (side, side), PUPIL), (0, 0), dot(iris * 0.48))
    glint = iris * 0.22
    gx, gy = cx + iris * 0.38, cy - iris * 0.4
    ImageDraw.Draw(layer).ellipse([gx - glint, gy - glint, gx + glint, gy + glint], fill=WHITE)
    return layer


def recoloured_background(icon: Image.Image, hue: int, value: float) -> Image.Image:
    """The artwork with its blue background turned to another hue; bottle, tape and pencil stay."""
    rgb = icon.convert("RGB")
    h, s, v = rgb.convert("HSV").split()
    blue = ImageChops.multiply(
        h.point(lambda x: 255 if 120 <= x <= 185 else 0), s.point(lambda x: 255 if x > 60 else 0)
    )
    shifted = Image.merge("HSV", (Image.new("L", rgb.size, hue), s, v.point(lambda x: round(x * value)))).convert("RGB")
    out = icon.copy()
    out.paste(shifted, (0, 0), blue)
    return out


def viewer_icon(icon: Image.Image) -> Image.Image:
    side = icon.size[0]
    out = recoloured_background(icon, hue=92, value=0.55)
    out.alpha_composite(big_eye(side, side * 0.5, side * 0.72, side * 0.88, side * 0.44, side * 0.035, IRIS))
    return out


def main() -> None:
    icon = transparent_corners(Image.open(SOURCE).convert("RGB"))
    write_icons(icon, ASSETS, "ketchup")
    VIEWER_ASSETS.mkdir(exist_ok=True)
    write_icons(viewer_icon(icon), VIEWER_ASSETS, "ketchup-viewer")


if __name__ == "__main__":
    main()
