"""Browser smoke test of the built web Viewer (scripts/windows/build-web-viewer.ps1).

Serves the site folder on 127.0.0.1 and, in each installed browser, checks that
the shared link opens the real model and draws it, that the embedded page does
the same inside an iframe, and that links to foreign origins or missing models
are refused with their error codes. Prints one line per check; exit code 1 on
any failure.

usage: python browser_smoke.py <site-folder> [chrome] [msedge] [firefox]
"""

import functools
import http.server
import io
import socket
import sys
import tempfile
import threading

from PIL import Image
from playwright.sync_api import sync_playwright

TIMEOUT_MS = 60_000


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def serve(folder):
    handler = functools.partial(QuietHandler, directory=folder)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{port}"


def status(frame, prefix):
    """Wait until the Viewer's canvas status starts with prefix; return it."""
    frame.wait_for_function(
        "p => (document.getElementById('ketchup-viewer')?.dataset.ketchupViewer || '').startsWith(p)",
        arg=prefix,
        timeout=TIMEOUT_MS,
    )
    return frame.eval_on_selector("#ketchup-viewer", "c => c.dataset.ketchupViewer")


def drawn(page, selector, save_as):
    """Geometry is drawn: besides the panel, the 3D background and its grid
    (the three most common colours) at least 2 % of the canvas is coloured."""
    shot = Image.open(io.BytesIO(page.locator(selector).screenshot(path=save_as))).convert("RGB")
    counts = sorted((count for count, _ in shot.getcolors(maxcolors=1 << 24)), reverse=True)
    return sum(counts[3:]) >= 0.02 * sum(counts)


def panel_left(page):
    """x of the information panel: where the dark 3D view ends, plus its margin.
    The Viewer draws its interface on the canvas, so buttons are found by layout."""
    shot = Image.open(io.BytesIO(page.screenshot())).convert("RGB")
    for x in range(shot.size[0] - 1, 0, -1):
        if sum(shot.getpixel((x, 400))) < 300:
            return x + 9
    raise AssertionError("no 3D view on the page")


def click(page, dx, y):
    page.mouse.click(panel_left(page) + dx, y)
    page.wait_for_timeout(400)


def orange_share(page):
    """Share of the view drawn in the selection highlight (orange)."""
    shot = Image.open(io.BytesIO(page.screenshot())).convert("RGB")
    pixels = list(shot.getdata())
    orange = sum(1 for r, g, b in pixels if r > 200 and 120 < g < 200 and b < 90)
    return orange / len(pixels)


def select_component(page, check):
    """Click a coloured component in the 3D view; it turns orange (selected)."""
    shot = Image.open(io.BytesIO(page.screenshot())).convert("RGB")
    right = panel_left(page) - 30
    target = next(
        (
            (x, y)
            for y in range(120, shot.size[1] - 20, 7)
            for x in range(20, right, 7)
            if (lambda r, g, b: max(r, g, b) - min(r, g, b) > 60)(*shot.getpixel((x, y)))
        ),
        None,
    )
    check("a component is visible to pick", target is not None)
    if target is None:
        return
    before = orange_share(page)
    page.mouse.click(*target)
    page.wait_for_timeout(500)
    after = orange_share(page)
    check("clicked component is selected", after > before + 0.001, f"{before:.4f} -> {after:.4f}")


def comment_round_trip(page, base, folder, name, check):
    """Write a comment, download it as .ketchup-comments, import it in a fresh page.
    Starts from a freshly opened link: a selection adds information above the
    comment buttons and would move them."""
    page.goto(f"{base}/index.html?model=model.ketchup-view")
    status(page, "ready")
    page.wait_for_timeout(800)
    click(page, 30, 131)  # Add comment
    click(page, 60, 147)  # Your name
    page.keyboard.type("Jana")
    click(page, 60, 200)  # Comment
    page.keyboard.type("Darker oak, please")
    click(page, 118, 253)  # Save comment
    check("comment written in the browser", status(page, "ready").count("comments=1") == 1)
    with page.expect_download() as download:
        click(page, 96, 251)  # Save comments for the author
    saved = download.value
    path = saved.path()
    saved.save_as(f"{folder}/{name}.ketchup-comments")
    check(
        "comments downloaded",
        saved.suggested_filename == "model.ketchup-comments" and path.stat().st_size > 0,
        saved.suggested_filename,
    )
    page.goto(f"{base}/index.html?model=model.ketchup-view")
    status(page, "ready")
    page.wait_for_timeout(800)
    with page.expect_file_chooser() as chooser:
        click(page, 50, 165)  # Open comments, with no comments listed yet
    chooser.value.set_files(str(path))
    imported = status(page, "ready components=20 scenes=1 comments=1")
    check("downloaded comments imported in a new page", "comments=1" in imported, imported)


BROWSERS = {
    "chrome": ("chromium", "chrome", ["--enable-unsafe-webgpu"]),
    "msedge": ("chromium", "msedge", ["--enable-unsafe-webgpu"]),
    # The same Chrome as a browser without WebGPU: the WebGL2 fallback path.
    "chrome-webgl": ("chromium", "chrome", []),
    "firefox": ("firefox", None, []),
    # Safari's engine as Playwright builds it for Windows; not Safari itself.
    "webkit": ("webkit", None, []),
    "chrome-phone": ("chromium", "chrome", []),
}


def launch(playwright, name):
    engine, channel, args = BROWSERS[name]
    if engine == "firefox":
        return playwright.firefox.launch()
    if engine == "webkit":
        return playwright.webkit.launch()
    return playwright.chromium.launch(channel=channel, args=args)


def run_phone(base, folder, playwright, check):
    """Phone-sized touch emulation in Chrome: the information moves below the
    model and a tap selects. An emulation, not a real Android or iPhone."""
    browser = playwright.chromium.launch(channel="chrome", args=["--enable-unsafe-webgpu"])
    page = browser.new_page(**playwright.devices["Pixel 7"])
    page.goto(f"{base}/index.html?model=model.ketchup-view")
    loaded = status(page, "ready")
    check("shared link opens the model on a phone screen", "components=" in loaded, loaded)
    page.wait_for_timeout(1500)
    check("model is drawn on a phone screen", drawn(page, "#ketchup-viewer", f"{folder}/shot-phone.png"))
    browser.close()


def run(base, folder, name, playwright, results):
    def check(label, ok, detail=""):
        results.append(ok)
        print(f"{'PASS' if ok else 'FAIL'} {name}: {label} {detail}".rstrip())

    if name == "chrome-phone":
        run_phone(base, folder, playwright, check)
        return
    browser = launch(playwright, name)
    page = browser.new_page(viewport={"width": 1100, "height": 760})
    if name == "chrome-webgl":
        page.add_init_script("delete Navigator.prototype.gpu")
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))

    page.goto(f"{base}/index.html?model=model.ketchup-view")
    loaded = status(page, "ready")
    check("shared link opens the model", "components=" in loaded, loaded)
    if name == "chrome-webgl":
        check("WebGL2 fallback is used", loaded.endswith("backend=gl"))
    page.wait_for_timeout(1500)
    check("model is drawn", drawn(page, "#ketchup-viewer", f"{folder}/shot-{name}.png"))
    select_component(page, check)
    comment_round_trip(page, base, folder, name, check)

    page.goto(f"{base}/embed-test.html")
    embedded = page.wait_for_selector("#ketchup-viewer-embed").content_frame()
    inner = status(embedded, "ready")
    check("embedded iframe opens the model", "components=" in inner, inner)
    page.wait_for_timeout(1500)
    check(
        "embedded model is drawn",
        drawn(page, "#ketchup-viewer-embed", f"{folder}/shot-{name}-embed.png"),
    )

    page.goto(f"{base}/index.html?model=https://evil.example/a.ketchup-view")
    check("foreign origin refused", status(page, "error") == "error viewer.link.origin")

    page.goto(f"{base}/index.html?model=missing.ketchup-view")
    check("missing model reported", status(page, "error") == "error viewer.link.fetch")

    page.goto(f"{base}/index.html")
    check("plain page waits for a file", status(page, "empty").startswith("empty"))

    check("no script errors", not errors, "; ".join(errors))
    browser.close()


def main():
    folder, names = sys.argv[1], sys.argv[2:] or list(BROWSERS)
    shots = tempfile.mkdtemp(prefix="ketchup-web-smoke-")
    print(f"screenshots: {shots}")
    server, base = serve(folder)
    results = []
    with sync_playwright() as playwright:
        for name in names:
            try:
                run(base, shots, name, playwright, results)
            except Exception as error:  # report the browser and continue with the next
                results.append(False)
                print(f"FAIL {name}: {type(error).__name__}: {error}".splitlines()[0])
    server.shutdown()
    sys.exit(0 if results and all(results) else 1)


if __name__ == "__main__":
    main()
