import { getCurrentWindow } from "@tauri-apps/api/window";
import { platform } from "@tauri-apps/plugin-os";
import { readable } from "svelte/store";
import { TAURI_ENV } from "./invoker";

// App windows draw their own title bar: on macOS the native one is overlaid
// (traffic lights stay native), elsewhere decorations are off entirely.

/** Whether the macOS traffic lights sit over the top-left of the window. */
export const MAC_TRAFFIC_LIGHTS = TAURI_ENV && platform() === "macos";

/** Whether the title bar has to draw minimize / maximize / close itself. */
export const CUSTOM_WINDOW_CONTROLS = TAURI_ENV && !MAC_TRAFFIC_LIGHTS;

/** Padding that clears the macOS traffic lights from the window's left edge. */
export const TRAFFIC_LIGHTS_INSET = 88;

/** Maximized / fullscreen state of the current window; static outside Tauri. */
export const windowState = readable(
  { maximized: false, fullscreen: false },
  (set) => {
    if (!TAURI_ENV) return;
    const appWindow = getCurrentWindow();
    const sync = async () => {
      const [maximized, fullscreen] = await Promise.all([
        appWindow.isMaximized(),
        appWindow.isFullscreen(),
      ]);
      set({ maximized, fullscreen });
    };
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void appWindow.onResized(() => void sync()).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    void sync();
    return () => {
      disposed = true;
      unlisten?.();
    };
  },
);

// Descendants of a drag region that keep their own mouse handling.
const INTERACTIVE =
  "button, a, input, select, textarea, label, [role='button'], [contenteditable], [data-no-drag]";

/**
 * Drag the window by this element and toggle maximize on double click, like a
 * native title bar. Unlike `data-tauri-drag-region`, this also covers
 * non-interactive descendants such as titles and labels.
 */
export function windowDrag(node: HTMLElement) {
  if (!TAURI_ENV) return {};

  const appWindow = getCurrentWindow();
  let pressX = 0;
  let pressY = 0;

  const draggable = (event: MouseEvent) =>
    event.button === 0 &&
    event.target instanceof Element &&
    !event.target.closest(INTERACTIVE);

  function onMouseDown(event: MouseEvent) {
    if (!draggable(event) || event.detail > 2) return;
    if (event.detail === 2 && MAC_TRAFFIC_LIGHTS) {
      // macOS zooms on mouseup, and only if the pointer did not move.
      pressX = event.clientX;
      pressY = event.clientY;
      return;
    }
    event.preventDefault();
    void (event.detail === 2
      ? appWindow.toggleMaximize()
      : appWindow.startDragging());
  }

  function onMouseUp(event: MouseEvent) {
    if (!MAC_TRAFFIC_LIGHTS || event.detail !== 2 || !draggable(event)) return;
    if (event.clientX === pressX && event.clientY === pressY) {
      void appWindow.toggleMaximize();
    }
  }

  node.addEventListener("mousedown", onMouseDown);
  node.addEventListener("mouseup", onMouseUp);
  return {
    destroy() {
      node.removeEventListener("mousedown", onMouseDown);
      node.removeEventListener("mouseup", onMouseUp);
    },
  };
}

/** Pause every <video> and <audio> in the page and drop its source. */
export function releaseMedia() {
  for (const media of document.querySelectorAll<HTMLMediaElement>("video, audio")) {
    media.pause();
    media.removeAttribute("src");
    media.load();
  }
}

/**
 * Run `beforeClose` when this window is closed; the window closes once it
 * settles. Tauri keeps a closed window's webview alive (wry retains the
 * WKWebView on purpose), so a page that is still playing would go on being
 * heard. Register one handler per window: every close-requested listener
 * destroys the window by itself. Returns the cleanup for `onMount`.
 */
export function onWindowClose(beforeClose: () => unknown): () => void {
  if (!TAURI_ENV) return () => {};
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void getCurrentWindow()
    .onCloseRequested(async () => {
      try {
        await beforeClose();
      } catch (error) {
        // Never keep the window open because cleanup failed.
        console.error("Window close cleanup failed:", error);
      }
    })
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
