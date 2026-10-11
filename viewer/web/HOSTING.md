# Web Kečup Viewer — hosting

The web Viewer is a static site: `index.html`, `main.js`, `viewer.css`,
`ketchup_viewer_web.js` and `ketchup_viewer_web_bg.wasm`, built by
`scripts/windows/build-web-viewer.ps1 -OutputDir <folder> [-Model <file.ketchup-view>]`.
It runs entirely in the customer's browser. Nothing is uploaded, no account is
needed and there is no server-side code; the host only serves files.

## Shared link

    https://example.com/viewer/index.html?model=designs/kitchen.ketchup-view

`model` is a path relative to the page (the same site) or an absolute `https`
address on an origin listed in the page's meta tag:

    <meta name="ketchup-viewer-allowed-origins" content="https://cdn.example.com">

Other origins, `data:`, `javascript:` and `file:` addresses are refused, so a
crafted link cannot show a foreign model under the manufacturer's address.
A model on another origin must also be served with
`Access-Control-Allow-Origin: https://example.com` (CORS); without it the
browser blocks the download and the Viewer shows `viewer.link.fetch`.

## Server settings

- Serve over HTTPS (WebGPU and downloads need a secure context; `localhost`
  counts as secure for testing).
- `.wasm` as `Content-Type: application/wasm`; `.ketchup-view` and
  `.ketchup-comments` as `application/octet-stream`.
- With a Content-Security-Policy, allow the module and WebAssembly:
  `script-src 'self' 'wasm-unsafe-eval'; connect-src 'self' <allowed model origins>`.
- `X-Content-Type-Options: nosniff`.

## Embedding

    <iframe src="https://example.com/viewer/index.html?model=designs/kitchen.ketchup-view"
            title="Kečup Viewer" allow="fullscreen"
            sandbox="allow-scripts allow-same-origin allow-downloads"></iframe>

`embed-test.html` is a working example. `allow-downloads` lets the customer
save comments; nothing else (camera, microphone, storage) is requested.
Limit who may embed the Viewer with `Content-Security-Policy: frame-ancestors
https://example.com https://partner.example`.

## Comments

The customer's comments stay in the page until they press *Save comments for
the author*, which downloads `<model>.ketchup-comments` (or *Save model copy
with comments*, a `.ketchup-view`). They send that file to the author, who
opens it in Kečup (*File → Open Viewer Comments…*). Leaving the page with
unsaved comments asks for confirmation.

## A public link is not protection

Anyone who has the link can open and download the model; the Viewer cannot
prevent it. For a confidential design, the host must restrict access to the
folder itself (login, expiring signed URLs, IP limits). Access control is the
host's responsibility; the Viewer never uploads or publishes anything by itself.

## Browser support (verified 2026-10-11, Windows)

| Browser | GPU path | Result |
| --- | --- | --- |
| Chrome (installed) | WebGPU | link, embed, drawing, part selection, comment download → import |
| Edge (installed) | WebGPU | same |
| Chrome without WebGPU | WebGL2 | same |
| Firefox 144 (Playwright build) | WebGL2 | same |
| WebKit 26 (Playwright build, Safari's engine) | WebGL2 | same |
| Chrome, Pixel 7 emulation (touch, phone screen) | WebGPU | link and drawing |

`viewer/web/tests/browser_smoke.py <site-folder>` repeats these checks. A
comment file downloaded in the browser opens in the desktop Viewer and in Kečup.
Scenes, dimensions and comment arrows run the same Rust code as the desktop
Viewer and are covered by its headless tests, not separately per browser.
Not verified here: Safari itself on macOS, Safari/Chrome on a real iPhone,
Chrome on a real Android device, and public HTTPS hosting.
