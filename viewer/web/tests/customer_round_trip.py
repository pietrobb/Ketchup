"""The customer's half of the V9d round trip in the built web Viewer.

Waits for the author's export `model.ketchup-view` in the exchange folder
(written by the ignored Rust test
`a_customer_comment_made_in_another_viewer_shows_the_author_the_customer_s_view_and_arrow`),
opens it through a shared link in Chrome, checks that the scene, its dimension
and the board's notes are shown, selects the board, turns the model, writes a
comment with an arrow onto the board and saves it for the author as
`returned.ketchup-comments` in the exchange folder. The Rust test then checks
what the author sees. Screenshots of every step stay in the exchange folder.

usage: python customer_round_trip.py <site-folder> <exchange-folder>
"""

import os
import shutil
import sys
import time

from browser_smoke import SAVE_PICKER, TIMEOUT_MS, panel_left, serve, status
from playwright.sync_api import sync_playwright
from PIL import Image

COMMENT = "Make the board 20 mm thicker"


def selected_point(page, path):
    """A point of the selected (orange) board nearest to the middle of the view."""
    page.screenshot(path=path)
    shot = Image.open(path).convert("RGB")
    right = panel_left(page) - 30
    centre = (right / 2, shot.size[1] / 2)
    points = [
        (x, y)
        for y in range(80, shot.size[1] - 20, 5)
        for x in range(20, right, 5)
        if (lambda r, g, b: r > 200 and 120 < g < 200 and b < 90)(*shot.getpixel((x, y)))
    ]
    assert points, "the selected board is not visible"
    return min(points, key=lambda p: (p[0] - centre[0]) ** 2 + (p[1] - centre[1]) ** 2)


def main():
    site, exchange = sys.argv[1], sys.argv[2]
    model = os.path.join(exchange, "model.ketchup-view")
    deadline = time.monotonic() + 600
    while not os.path.exists(model):
        assert time.monotonic() < deadline, "no export from the author"
        time.sleep(0.5)
    shutil.copy(model, os.path.join(site, "model.ketchup-view"))
    server, base = serve(site)
    failures = []

    def check(label, ok, detail=""):
        print(f"{'PASS' if ok else 'FAIL'} web: {label} {detail}".rstrip())
        if not ok:
            failures.append(label)

    def shot(name):
        page.screenshot(path=os.path.join(exchange, f"web-{name}.png"))

    def panel(dx, y):
        page.mouse.click(panel_left(page) + dx, y)
        page.wait_for_timeout(400)

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel="chrome", args=["--enable-unsafe-webgpu"])
        page = browser.new_page(viewport={"width": 1100, "height": 760}, locale="en-US")
        page.add_init_script(SAVE_PICKER)
        page.on("dialog", lambda dialog: dialog.accept())
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))

        page.goto(f"{base}/index.html?model=model.ketchup-view")
        opened = status(page, "ready")
        check("the shared link opens the author's export", "scenes=1" in opened, opened)
        page.wait_for_timeout(1500)
        shot("1-scene")
        page.mouse.click(430, 400)  # the board, in the author's scene
        page.wait_for_timeout(600)
        shot("2-board")

        # Turned to a view of their own.
        page.mouse.move(300, 600)
        page.mouse.down()
        for step in range(1, 7):
            page.mouse.move(300 + 12 * step, 600 - 5 * step)
            page.wait_for_timeout(30)
        page.mouse.up()
        page.wait_for_timeout(400)
        tip = selected_point(page, os.path.join(exchange, "web-3-turned.png"))
        panel(64, 247)  # Add comment
        shot("4-form")
        panel(110, 264)  # Your name
        page.keyboard.type("Jana")
        panel(110, 316)  # Comment
        page.keyboard.type(COMMENT)
        panel(44, 370)  # Add arrow
        page.mouse.click(*tip)
        page.wait_for_timeout(300)
        page.mouse.click(60, 100)
        page.wait_for_timeout(400)
        shot("5-arrow")
        panel(52, 391)  # Save comment, below Remove arrows once there is one
        written = status(page, "ready")
        check("the comment is kept", "comments=1" in written, written)
        shot("6-saved")
        page.evaluate("() => { window.__picker.cancel = false; }")
        panel(95, 368)  # Save comments for the author
        page.wait_for_function("() => window.__picker.saved !== null", timeout=TIMEOUT_MS)
        saved = page.evaluate("() => window.__picker")
        check("comments saved for the author", bool(saved["saved"]), saved["name"])
        part = os.path.join(exchange, "returned.ketchup-comments.part")
        with open(part, "wb") as file:
            file.write(bytes(saved["saved"]))
        os.replace(part, os.path.join(exchange, "returned.ketchup-comments"))
        check("no script errors", not errors, "; ".join(errors))
        browser.close()
    server.shutdown()
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
