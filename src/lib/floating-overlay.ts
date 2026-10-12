/** Own floating nodes until they finish, are cleared, or the player exits. */
export function createFloatingOverlay(overlay: HTMLElement) {
  const cleanups = new Set<() => void>();
  let destroyed = false;

  function add(node: HTMLElement, disposeContent: () => void = () => {}) {
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
    overlay.appendChild(node);
    frame = requestAnimationFrame(() => {
      frame = undefined;
      if (released) return;
      node.style.transform = `translateX(-${overlay.clientWidth + node.clientWidth}px)`;
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
