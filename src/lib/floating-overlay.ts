/**
 * Where a node flies: its left edge moves `travel` leftwards from `entryX` in
 * `duration` seconds, of which `elapsed` have passed already.
 */
export interface FloatingFlight {
  top: number;
  entryX: number;
  travel: number;
  duration: number;
  elapsed?: number;
}

/**
 * Own floating nodes until they finish, are cleared, or the player exits.
 * Those added to `follow` playback move with the state set with
 * `setPlayback`; the rest at their own pace.
 */
export function createFloatingOverlay(overlay: HTMLElement) {
  const cleanups = new Set<() => void>();
  const animations = new Set<Animation>();
  let playback = { paused: false, rate: 1 };
  let destroyed = false;

  /**
   * Fly `node`. `place` gets its size once it is laid out, and says where it
   * flies, or nothing for no room: then it is dropped, or with `keep`, asked
   * again every frame.
   */
  function add(
    node: HTMLElement,
    place: (width: number, height: number) => FloatingFlight | null,
    { dispose: disposeContent = () => {}, keep = false, follow = false } = {},
  ) {
    if (destroyed) {
      disposeContent();
      node.remove();
      return () => {};
    }

    let released = false;
    let frame: number | undefined;
    let animation: Animation | undefined;
    const dispose = () => {
      if (released) return;
      released = true;
      cleanups.delete(dispose);
      if (frame !== undefined) cancelAnimationFrame(frame);
      if (animation) {
        animations.delete(animation);
        animation.onfinish = null;
        animation.cancel();
      }
      disposeContent();
      node.remove();
    };
    cleanups.add(dispose);
    // Laid out unseen, with the player's width to fit in, until placed.
    node.style.position = "absolute";
    node.style.left = "0";
    node.style.top = "0";
    node.style.visibility = "hidden";
    overlay.appendChild(node);
    const fly = () => {
      frame = undefined;
      if (released) return;
      const flight = place(node.offsetWidth, node.offsetHeight);
      if (!flight) {
        if (keep) frame = requestAnimationFrame(fly);
        else dispose();
        return;
      }
      const elapsed = Math.max(0, flight.elapsed ?? 0);
      if (elapsed >= flight.duration) {
        dispose();
        return;
      }
      node.style.left = `${flight.entryX}px`;
      node.style.top = `${flight.top}px`;
      node.style.visibility = "";
      animation = node.animate(
        [{ transform: "translateX(0)" }, { transform: `translateX(-${flight.travel}px)` }],
        { duration: flight.duration * 1000, easing: "linear" },
      );
      animation.currentTime = elapsed * 1000;
      animation.onfinish = dispose;
      if (follow) {
        animation.playbackRate = playback.rate;
        if (playback.paused) animation.pause();
        animations.add(animation);
      }
    };
    frame = requestAnimationFrame(fly);
    return dispose;
  }

  /** Pause, resume or speed up the nodes that follow playback, now and to come. */
  function setPlayback(paused: boolean, rate: number) {
    playback = { paused, rate };
    for (const animation of animations) {
      animation.playbackRate = rate;
      if (paused) animation.pause();
      else animation.play();
    }
  }

  function clear() {
    for (const dispose of [...cleanups]) dispose();
  }

  function destroy() {
    destroyed = true;
    clear();
    overlay.remove();
  }

  return { add, setPlayback, clear, destroy };
}

/** How long a resize must settle before danmaku are laid out again. */
const RESIZE_SETTLE_MS = 150;

/**
 * Call `relayout` once `overlay` settles at a new size: danmaku on screen
 * keep the positions and rows they were placed with in the old one.
 */
export function observeResize(overlay: HTMLElement, relayout: () => void) {
  let size = `${overlay.clientWidth}x${overlay.clientHeight}`;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const observer = new ResizeObserver(() => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const next = `${overlay.clientWidth}x${overlay.clientHeight}`;
      if (next === size) return;
      size = next;
      relayout();
    }, RESIZE_SETTLE_MS);
  });
  observer.observe(overlay);
  return () => {
    clearTimeout(timer);
    observer.disconnect();
  };
}
