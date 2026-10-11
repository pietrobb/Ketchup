// Starts the Kečup Viewer (WebAssembly) on the page's canvas. The model comes
// from the shared link, e.g. index.html?model=designs/kitchen.ketchup-view
import init, { start } from "./ketchup_viewer_web.js";

const canvas = document.getElementById("ketchup-viewer");
const status = document.getElementById("ketchup-viewer-status");
const origins = document.querySelector('meta[name="ketchup-viewer-allowed-origins"]');

try {
  await init();
  status.textContent = "";
  await start(canvas, origins ? origins.content : "");
} catch (error) {
  status.textContent =
    "Kečup Viewer sa v tomto prehliadači nedá spustiť (potrebuje WebAssembly a WebGPU alebo WebGL2). " +
    String(error);
}
