"""Builds the application icon files from the artwork in crates/ketchup-app/assets.

Writes ketchup.ico (the Windows executable and installer icon) and
ketchup-128.rgba (the window icon: 128 x 128 straight RGBA, row-major).
The Viewer gets the same artwork with an eye badge in the lower right corner,
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
# The eye badge spans this share of the icon side, large enough to read at 16 px.
BADGE_SHARE = 0.46
BADGE_SUPERSAMPLE = 4
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


def disk(side: int, center: float, radius: float) -> Image.Image:
    mask = Image.new("L", (side, side), 0)
    ImageDraw.Draw(mask).ellipse([center - radius, center - radius, center + radius, center + radius], fill=255)
    return mask


def eye_badge(side: int) -> Image.Image:
    """A navy-rimmed white disk holding an almond eye with a blue iris."""
    n = side * BADGE_SUPERSAMPLE
    center = n / 2
    badge = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    draw = ImageDraw.Draw(badge)
    rim = n * 0.07
    draw.ellipse([0, 0, n - 1, n - 1], fill=NAVY)
    draw.ellipse([rim, rim, n - 1 - rim, n - 1 - rim], fill=WHITE)
    # The almond is the overlap of two circles through both eye corners.
    width, height = n * 0.70, n * 0.42
    radius = (width * width / 4 + height * height / 4) / height
    upper = Image.new("L", (n, n), 0)
    ImageDraw.Draw(upper).ellipse(
        [center - radius, center - height / 2, center + radius, center - height / 2 + 2 * radius], fill=255
    )
    lower = Image.new("L", (n, n), 0)
    ImageDraw.Draw(lower).ellipse(
        [center - radius, center + height / 2 - 2 * radius, center + radius, center + height / 2], fill=255
    )
    almond = ImageChops.multiply(upper, lower)
    badge.paste(Image.new("RGBA", (n, n), NAVY), (0, 0), almond)
    inset = int(n * 0.05)
    sclera = Image.new("L", (n, n), 0)
    sclera.paste(almond.resize((n - 2 * inset, n - 2 * inset)), (inset, inset))
    badge.paste(Image.new("RGBA", (n, n), WHITE), (0, 0), sclera)
    iris = n * 0.16
    badge.paste(Image.new("RGBA", (n, n), IRIS), (0, 0), disk(n, center, iris))
    badge.paste(Image.new("RGBA", (n, n), PUPIL), (0, 0), disk(n, center, n * 0.075))
    glint = n * 0.035
    draw.ellipse(
        [center + iris * 0.35 - glint, center - iris * 0.4 - glint, center + iris * 0.35 + glint, center - iris * 0.4 + glint],
        fill=WHITE,
    )
    return badge.resize((side, side), Image.Resampling.LANCZOS)


def with_eye_badge(icon: Image.Image) -> Image.Image:
    side = icon.size[0]
    badge_side = int(side * BADGE_SHARE)
    viewer = icon.copy()
    viewer.alpha_composite(eye_badge(badge_side), (side - badge_side, side - badge_side))
    return viewer


def write_icons(icon: Image.Image, folder: Path, stem: str) -> None:
    largest = scaled(icon, ICO_SIZES[0])
    largest.save(
        folder / f"{stem}.ico",
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
        append_images=[scaled(icon, size) for size in ICO_SIZES[1:]],
    )
    (folder / f"{stem}-{WINDOW_SIZE}.rgba").write_bytes(scaled(icon, WINDOW_SIZE).tobytes())


def main() -> None:
    icon = transparent_corners(Image.open(SOURCE).convert("RGB"))
    write_icons(icon, ASSETS, "ketchup")
    VIEWER_ASSETS.mkdir(exist_ok=True)
    write_icons(with_eye_badge(icon), VIEWER_ASSETS, "ketchup-viewer")


if __name__ == "__main__":
    main()
