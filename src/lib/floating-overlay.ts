/** Where a node flies: its left edge moves `travel` leftwards from `entryX` in `duration` seconds. */
export interface FloatingFlight {
  top: number;
  entryX: number;
  travel: number;
  duration: number;
}

/**
 * Own floating nodes until they finish, are cleared, or the player exits.
 * `place` gets a node's size once it is laid out, and whether it must be
 * kept, and says where it flies, or nothing to drop it.
 */
export function createFloatingOverlay(
  overlay: HTMLElement,
  place: (width: number, height: number, keep: boolean) => FloatingFlight | null,
) {
  const cleanups = new Set<() => void>();
  let destroyed = false;

  function add(node: HTMLElement, disposeContent: () => void = () => {}, keep = false) {
    if (destroyed) {
      disposeContent();
      node.remove();
      return () => {};
    }

    let released = false;
    let frame: number | undefined;
    const dispose = () => {
      if (released) return;
      released = true;
      cleanups.delete(dispose);
      if (frame !== undefined) cancelAnimationFrame(frame);
      node.removeEventListener("transitionend", onTransitionEnd);
      node.removeEventListener("transitioncancel", onTransitionEnd);
      disposeContent();
      node.remove();
    };
    const onTransitionEnd = (event: TransitionEvent) => {
      if (event.target === node && event.propertyName === "transform") dispose();
    };
    cleanups.add(dispose);
    node.addEventListener("transitionend", onTransitionEnd);
    node.addEventListener("transitioncancel", onTransitionEnd);
    // Laid out unseen, with the player's width to fit in, until placed.
    node.style.position = "absolute";
    node.style.left = "0";
    node.style.top = "0";
    node.style.visibility = "hidden";
    overlay.appendChild(node);
    frame = requestAnimationFrame(() => {
      frame = undefined;
      if (released) return;
      // Reading the size also lays out the starting position to move from.
      const flight = place(node.offsetWidth, node.offsetHeight, keep);
      if (!flight) {
        dispose();
        return;
      }
      node.style.left = `${flight.entryX}px`;
      node.style.top = `${flight.top}px`;
      node.style.visibility = "";
      node.style.transition = `transform ${flight.duration}s linear`;
      node.style.transform = `translateX(-${flight.travel}px)`;
    });
    return dispose;
  }

  function clear() {
    for (const dispose of [...cleanups]) dispose();
  }

  function destroy() {
    destroyed = true;
    clear();
    overlay.remove();
  }

  return { add, clear, destroy };
}
